import Foundation
import Observation
import UIKit
import WebKit

/// One open ledger: Ledgeline's in-process server for it and the web page
/// showing it. Dropping it drops the `LedgelineApp` first, then the
/// security-scoped access the app read through.
@MainActor
@Observable
final class LedgerSession {
    let ledgerID: UUID
    let app: LedgelineApp
    let page: WebPage
    let info: PageInfo
    /// Why the last refresh could not load the journal (it no longer parses);
    /// the last good snapshot is still on screen.
    var refreshProblem: String?

    /// Held for the session's lifetime: dropping it ends the access the
    /// `LedgelineApp` reads through. Declared after `app` so it outlives it.
    private let access: ScopedAccess
    private let decider: LinkPolicy
    private var watcher: Task<Void, Never>?
    private var pendingRefresh: Task<Void, Never>?
    private var refreshing = false
    /// The home indicator's height, pt (`LedgerScreen` measures it).
    private var bottomInset: Double = 0

    // MARK: Opening

    /// Opens `ledger`. Everything slow (resolving the bookmark, parsing the
    /// journal) runs off the main thread.
    static func open(
        _ ledger: Ledger,
        store: LedgerStore,
        coordinator: FileCoordinatorBridge
    ) async throws -> LedgerSession {
        let access = try await store.access(ledger)
        let start = ContinuousClock.now
        let cacheDir = AppPaths.cacheFolder(for: ledger.id).path(percentEncoded: false)
        let configDir = AppPaths.configFolder.path(percentEncoded: false)
        let root = access.path
        let app: LedgelineApp = try await Task.detached(priority: .userInitiated) {
            switch ledger.scope {
            case .file:
                return try LedgelineApp.openFile(path: root, configDir: configDir, coordinator: coordinator)
            case .folder:
                return try LedgelineApp.open(
                    rootPath: root, mainFile: ledger.mainFile, cacheDir: cacheDir, configDir: configDir,
                    coordinator: coordinator
                )
            case .sample:
                // Inside the app's own container: plain writes are right.
                return try LedgelineApp.open(
                    rootPath: root, mainFile: ledger.mainFile, cacheDir: cacheDir, configDir: configDir,
                    coordinator: nil
                )
            }
        }.value
        Log.info("open: \(ledger.scope.rawValue) ledger in \(millis(since: start))ms "
            + "(editable=\(app.isEditable()))")
        return LedgerSession(ledgerID: ledger.id, app: app, access: access)
    }

    private init(ledgerID: UUID, app: LedgelineApp, access: ScopedAccess) {
        self.ledgerID = ledgerID
        self.app = app
        self.access = access
        let info = PageInfo()
        let decider = LinkPolicy()
        var configuration = WebPage.Configuration()
        configuration.urlSchemeHandlers[URLScheme(LedgelineSchemeHandler.scheme)!] = LedgelineSchemeHandler(app: app)
        NativeBridge.install(
            in: &configuration,
            tokenScript: app.tokenInitScript(origin: nil),
            receiver: NativeMessageReceiver(info: info)
        )
        let page = WebPage(configuration: configuration, navigationDecider: decider)
        #if DEBUG
            page.isInspectable = true
        #endif
        decider.page = page
        self.page = page
        self.info = info
        self.decider = decider
    }

    /// Loads the SPA and starts watching for a crashed WebContent process.
    func start() async {
        self.watcher = Task { [weak self] in await self?.watchNavigations() }
        #if DEBUG
            await self.load(Self.debugStartURL(), label: "first page")
            await self.runDebugScript()
        #else
            await self.load(LedgelineSchemeHandler.origin, label: "first page")
        #endif
    }

    #if DEBUG
        /// Screenshots without a tap: `SIMCTL_CHILD_LEDGELINE_START_PATH=
        /// "/reports?tab=balance-sheet"` opens that page first.
        private static func debugStartURL() -> URL {
            guard let path = ProcessInfo.processInfo.environment["LEDGELINE_START_PATH"], path.hasPrefix("/"),
                  let url = URL(string: path, relativeTo: LedgelineSchemeHandler.origin)?.absoluteURL
            else { return LedgelineSchemeHandler.origin }
            return url
        }

        /// `SIMCTL_CHILD_LEDGELINE_DEBUG_JS="…"`: evaluated once after the first
        /// load (and a short settle), e.g. to open a drawer or an overlay.
        private func runDebugScript() async {
            guard let script = ProcessInfo.processInfo.environment["LEDGELINE_DEBUG_JS"], !script.isEmpty else { return }
            try? await Task.sleep(for: .milliseconds(1500))
            do {
                let result = try await self.page.callJavaScript(script)
                Log.info("debug js: \(String(describing: result))")
            } catch {
                Log.info("debug js failed: \(error)")
            }
        }
    #endif

    /// The web view runs under the home indicator, so the page cannot see
    /// it; tell it (and tell every new document, `applyInsets`).
    func setBottomInset(_ points: CGFloat) {
        let value = Double(points)
        guard value != self.bottomInset else { return }
        self.bottomInset = value
        Task { await self.applyInsets() }
    }

    private func applyInsets() async {
        await NativeBridge.setBottomInset(self.bottomInset, in: self.page)
    }

    func close() {
        self.watcher?.cancel()
        self.watcher = nil
        self.pendingRefresh?.cancel()
        self.pendingRefresh = nil
    }

    @discardableResult
    func load(_ url: URL, label: String) async -> Bool {
        let start = ContinuousClock.now
        do {
            for try await event in self.page.load(URLRequest(url: url)) where event == .finished {
                Log.info("\(label): finished in \(millis(since: start))ms")
                await self.applyInsets()
                return true
            }
        } catch {
            Log.info("\(label): \(url.path()) failed: \(error)")
        }
        return false
    }

    /// Reloads the page on screen (or the SPA's root, when there is none).
    func reload(why: String) async {
        guard self.page.url != nil else {
            await self.load(LedgelineSchemeHandler.origin, label: "reload (\(why))")
            return
        }
        Log.info("reload: \(why)")
        self.info.reset()
        do {
            for try await event in self.page.reload() where event == .finished {
                await self.applyInsets()
                break
            }
        } catch {
            Log.info("reload: failed: \(error)")
        }
    }

    // MARK: Freshness

    /// Picks up changes made outside the app (foreground, the Refresh menu
    /// item, a pending download landing). `Reloaded` reloads the page;
    /// `Pending` tries again shortly; `Failed` keeps the last good snapshot
    /// and says why.
    func refresh(reason: String) async {
        guard !self.refreshing else { return }
        self.refreshing = true
        defer { self.refreshing = false }
        do {
            let summary = try await app.refresh()
            Log.info("refresh (\(reason)): \(summary.outcome) in \(summary.durationMs)ms "
                + "notDownloaded=\(summary.notDownloaded.count) editable=\(summary.editable)")
            switch summary.outcome {
            case .unchanged:
                self.refreshProblem = nil
            case .reloaded:
                self.refreshProblem = nil
                await self.reload(why: "journal changed")
            case .pending:
                self.retryRefresh(after: .seconds(3))
            case .failed:
                self.refreshProblem = summary.message ?? "The journal could not be read."
            }
        } catch {
            Log.info("refresh (\(reason)): failed: \(error)")
        }
    }

    /// Files are downloading: refresh again once they may have landed.
    private func retryRefresh(after delay: Duration) {
        self.pendingRefresh?.cancel()
        self.pendingRefresh = Task { [weak self] in
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            await self?.refresh(reason: "downloads")
        }
    }

    // MARK: Recovery

    /// Follows every navigation. WebKit kills a WebContent process under
    /// memory pressure (and reclaims backgrounded ones); the page would stay
    /// blank, so it is reloaded.
    private func watchNavigations() async {
        while !Task.isCancelled {
            do {
                for try await _ in self.page.navigations {}
                return
            } catch WebPage.NavigationError.webContentProcessTerminated {
                Log.info("page: web content process terminated; reloading")
                self.info.reset()
                await self.load(self.page.url ?? LedgelineSchemeHandler.origin, label: "reload after crash")
            } catch {
                // A failed navigation ends this sequence; keep watching.
                Log.info("page: navigation failed: \(error)")
            }
        }
    }
}

/// Where a navigation may go. The SPA stays on `ledgeline://localhost`;
/// http(s) and anything else opens in the system's handler (Safari, Mail…);
/// `blob:`, `data:` and `about:` load in place.
@MainActor
private final class LinkPolicy: WebPage.NavigationDeciding {
    weak var page: WebPage?

    func decidePolicy(
        for action: WebPage.NavigationAction,
        preferences _: inout WebPage.NavigationPreferences
    ) async -> WKNavigationActionPolicy {
        guard let url = action.request.url, let scheme = url.scheme?.lowercased() else { return .allow }
        switch scheme {
        case LedgelineSchemeHandler.scheme:
            guard url.host()?.lowercased() == LedgelineSchemeHandler.host else {
                Log.info("link: refused ledgeline:// host \(url.host() ?? "-")")
                return .cancel
            }
            // `target="_blank"`: no tabs to open it in, so open it here.
            if action.target == nil, let page {
                Task {
                    do {
                        for try await event in page.load(URLRequest(url: url)) where event == .finished {
                            break
                        }
                    } catch {
                        Log.info("link: \(url.path()) failed: \(error)")
                    }
                }
                return .cancel
            }
            return .allow
        case "about", "data", "blob":
            return .allow
        default:
            Log.info("link: opening a \(scheme): link outside the app")
            await UIApplication.shared.open(url)
            return .cancel
        }
    }
}
