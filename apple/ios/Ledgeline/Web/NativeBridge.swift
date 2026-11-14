import Foundation
import Observation
import UIKit
import WebKit

// The native bridge contract (plans/42-mobile-app.md, "The native bridge
// contract"). Every wire literal lives in this file, spelled out: a vitest on
// the SPA side reads it and checks each one against `web/src/lib/native/`, so
// a rename on either side fails a test instead of silently emptying the bar.
// swiftformat:disable redundantRawValues

/// `type` of a page → Swift message.
enum NativeMessageType: String {
    case page = "page"
    case share = "share"
    case haptic = "haptic"
}

/// `sections[].id` the page may report. Imports is hidden in native mode.
enum NativeSectionID: String, CaseIterable {
    case journal = "journal"
    case holdings = "holdings"
    case budget = "budget"
    case projections = "projections"
    case reports = "reports"
    case settings = "settings"

    /// The menu's icon for a section.
    var symbol: String {
        switch self {
        case .journal: "list.bullet.rectangle"
        case .holdings: "chart.pie"
        case .budget: "dollarsign.gauge.chart.leftthird.topthird.rightthird"
        case .projections: "chart.line.uptrend.xyaxis"
        case .reports: "tablecells"
        case .settings: "gearshape"
        }
    }
}

/// `haptic` message styles.
enum NativeHapticStyle: String {
    case light = "light"
    case success = "success"
    case warning = "warning"
}

// swiftformat:enable redundantRawValues

/// One entry of `page.sections`.
struct PageSection: Identifiable, Hashable {
    let id: String
    let label: String
    let active: Bool

    var symbol: String? {
        NativeSectionID(rawValue: self.id)?.symbol
    }
}

/// One entry of `page.actions`. `icon` is an SF Symbol name.
struct PageAction: Identifiable, Hashable {
    let id: String
    let label: String
    let icon: String?
    let destructive: Bool
}

/// The last `page` message: what the native bar shows.
@MainActor
@Observable
final class PageInfo {
    private(set) var title = ""
    private(set) var sections: [PageSection] = []
    private(set) var actions: [PageAction] = []
    private(set) var canGoBack = false
    /// Whether the page has reported at all (the SPA's bridge is live).
    private(set) var reported = false

    /// Applies a `{type: "page", title, sections, actions, canGoBack}`
    /// message. Malformed entries are dropped; the bar keeps the rest.
    func apply(page message: [String: Any]) {
        self.reported = true
        self.title = (message["title"] as? String) ?? ""
        self.sections = ((message["sections"] as? [[String: Any]]) ?? []).compactMap { section in
            guard let id = section["id"] as? String, let label = section["label"] as? String else { return nil }
            return PageSection(id: id, label: label, active: (section["active"] as? Bool) ?? false)
        }
        self.actions = ((message["actions"] as? [[String: Any]]) ?? []).compactMap { action in
            guard let id = action["id"] as? String, let label = action["label"] as? String else { return nil }
            let icon = (action["icon"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            return PageAction(id: id, label: label, icon: icon, destructive: (action["destructive"] as? Bool) ?? false)
        }
        self.canGoBack = (message["canGoBack"] as? Bool) ?? false
    }

    /// A new document is loading: what the old one reported no longer holds.
    func reset() {
        self.reported = false
        self.canGoBack = false
    }
}

/// Wires a `WebPage.Configuration` for the shell: the document-start scripts
/// and the `ledgeline` message handler; and calls into the page's
/// `window.ledgelineNative`.
enum NativeBridge {
    /// The script-message handler the page posts to:
    /// `window.webkit.messageHandlers.ledgeline.postMessage(msg)`.
    static let messageHandler = "ledgeline"

    /// `"pad"` on an iPad, else `"phone"`. Layout keys off the width, never
    /// this; it is informational.
    @MainActor
    static var idiom: String {
        UIDevice.current.userInterfaceIdiom == .pad ? "pad" : "phone"
    }

    /// The page's UI-scale event (`ZOOM_EVENT` in web/src/lib/theme/zoom.svelte.ts).
    static let zoomEvent = "ledgeline:zoom"

    /// The custom property the page reads the home indicator's height from
    /// (`--ll-bottom-inset` in web/src/app.css).
    static let bottomInsetProperty = "--ll-native-bottom"

    /// Runs at document start in the main frame, after `tokenInitScript()`.
    /// Also turns WebKit's page zoom off once the viewport tag is parsed: a
    /// pinch steps the SPA's own UI scale instead (`PinchToScale`), and a
    /// focused small input no longer zooms the page.
    static func nativeScript(idiom: String) -> String {
        """
        window.__LEDGELINE_NATIVE__ = {platform: "ios", idiom: "\(idiom)", version: 1};
        document.documentElement.classList.add("ll-native");
        document.addEventListener("DOMContentLoaded", function () {
            var meta = document.querySelector('meta[name="viewport"]');
            if (meta && meta.content.indexOf("user-scalable") < 0) {
                meta.content += ", maximum-scale=1, user-scalable=no";
            }
        });
        """
    }

    /// Installs the token script, then the native script (both document
    /// start, main frame only), and the message handler.
    @MainActor
    static func install(
        in configuration: inout WebPage.Configuration,
        tokenScript: String,
        receiver: NativeMessageReceiver
    ) {
        let controller = configuration.userContentController
        controller.addUserScript(WKUserScript(
            source: tokenScript, injectionTime: .atDocumentStart, forMainFrameOnly: true
        ))
        controller.addUserScript(WKUserScript(
            source: self.nativeScript(idiom: self.idiom), injectionTime: .atDocumentStart, forMainFrameOnly: true
        ))
        controller.add(receiver, name: self.messageHandler)
    }

    // MARK: Swift → page

    /// `window.ledgelineNative.navigate(sectionId)`.
    @MainActor
    static func navigate(_ sectionID: String, in page: WebPage) async -> Bool {
        await self.call("return window.ledgelineNative ? window.ledgelineNative.navigate(id) : false",
                        id: sectionID, in: page)
    }

    /// `window.ledgelineNative.action(actionId)`.
    @MainActor
    static func action(_ actionID: String, in page: WebPage) async -> Bool {
        await self.call("return window.ledgelineNative ? window.ledgelineNative.action(id) : false",
                        id: actionID, in: page)
    }

    /// `window.ledgelineNative.back()`: `false` (or no bridge) means nothing
    /// to go back to, and the app shows the ledger list.
    @MainActor
    static func back(in page: WebPage) async -> Bool {
        await self.call("return window.ledgelineNative ? window.ledgelineNative.back() : false", id: nil, in: page)
    }

    /// One UI-scale step: the same `ledgeline:zoom` event the desktop menu
    /// dispatches, so the scale is stepped, applied and saved as there.
    @MainActor
    static func zoom(_ direction: PinchSteps.Direction, in page: WebPage) async {
        do {
            _ = try await page.callJavaScript(
                "window.dispatchEvent(new CustomEvent(name, {detail: detail})); return true",
                arguments: ["name": self.zoomEvent, "detail": direction.rawValue]
            )
        } catch {
            Log.info("bridge: zoom failed: \(error)")
        }
    }

    /// Hands the page the home indicator's height (`--ll-native-bottom`).
    @MainActor
    static func setBottomInset(_ points: Double, in page: WebPage) async {
        do {
            _ = try await page.callJavaScript(
                "document.documentElement.style.setProperty(name, value + 'px'); return true",
                arguments: ["name": self.bottomInsetProperty, "value": points]
            )
        } catch {
            Log.info("bridge: bottom inset failed: \(error)")
        }
    }

    /// `window.ledgelineNative.state()`: the last `page` payload, to resync
    /// the bar (after a reload, say).
    @MainActor
    static func state(in page: WebPage) async -> [String: Any]? {
        do {
            let state = try await page.callJavaScript(
                "return window.ledgelineNative ? window.ledgelineNative.state() : null"
            )
            return state as? [String: Any]
        } catch {
            Log.info("bridge: state() failed: \(error)")
            return nil
        }
    }

    @MainActor
    private static func call(_ body: String, id: String?, in page: WebPage) async -> Bool {
        do {
            let result = try await page.callJavaScript(body, arguments: id.map { ["id": $0] } ?? [:])
            return (result as? Bool) ?? false
        } catch {
            Log.info("bridge: \(body) failed: \(error)")
            return false
        }
    }
}

/// Receives the page's messages. WebKit holds it strongly; it holds the page
/// info weakly, so a closed ledger leaves nothing behind.
final class NativeMessageReceiver: NSObject, WKScriptMessageHandler {
    private weak var info: PageInfo?

    @MainActor
    init(info: PageInfo) {
        self.info = info
    }

    func userContentController(_: WKUserContentController, didReceive message: WKScriptMessage) {
        // Main frame only: an embedded frame has no say over the native bar.
        guard message.frameInfo.isMainFrame,
              message.frameInfo.securityOrigin.protocol == LedgelineSchemeHandler.scheme
        else { return }
        let body = message.body
        MainActor.assumeIsolated {
            self.receive(body)
        }
    }

    @MainActor
    private func receive(_ body: Any) {
        guard let message = body as? [String: Any],
              let type = (message["type"] as? String).flatMap(NativeMessageType.init(rawValue:))
        else {
            Log.info("bridge: ignored a message with no known type")
            return
        }
        switch type {
        case .page:
            self.info?.apply(page: message)
        case .share:
            guard let filename = message["filename"] as? String, let base64 = message["base64"] as? String else {
                Log.info("bridge: share without filename/base64; ignored")
                return
            }
            ShareSheet.present(
                filename: filename,
                mime: (message["mime"] as? String) ?? "application/octet-stream",
                base64: base64
            )
        case .haptic:
            Haptics.play((message["style"] as? String) ?? NativeHapticStyle.light.rawValue)
        }
    }
}
