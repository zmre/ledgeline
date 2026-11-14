import SwiftUI
import WebKit

/// One open ledger: the SPA in a web view, under a native bar whose title,
/// back button and hamburger menu follow what the page reports
/// (`NativeBridge`).
struct LedgerScreen: View {
    let route: Route
    @Environment(AppModel.self) private var model
    @Environment(LedgerStore.self) private var store
    @State private var session: LedgerSession?
    @State private var error: String?
    /// `openFile` refused a journal with includes: the folder is needed.
    @State private var needsFolder: String?
    /// Files the journal needs are still downloading.
    @State private var downloading = false
    @State private var pickedFolder: PickedFolder?
    /// The owner cancelled (or failed) the unlock prompt for this ledger.
    @State private var lockedOut = false

    var body: some View {
        self.content
            .navigationTitle(self.pageTitle)
            // The ledger's name, unless the title already is (before the page reports).
            .navigationSubtitle(self.session?.info.title.isEmpty == false ? self.ledgerName : "")
            .navigationBarTitleDisplayMode(.inline)
            // Back is the page's (overlay, then in-app history), then home.
            .navigationBarBackButtonHidden(true)
            .toolbar { self.toolbar }
            .sheet(item: self.$pickedFolder) { folder in
                JournalChooser(folder: folder) { mainFile in
                    Task { await self.becomeFolder(folder, mainFile: mainFile) }
                }
            }
            .task(id: self.route) { await self.open() }
            .onDisappear {
                if let session { self.model.close(session) }
            }
    }

    @ViewBuilder
    private var content: some View {
        if let session {
            WebView(session.page)
                // Back is the bar's button; WebKit's edge swipes would walk the
                // web history behind the SPA's back.
                .webViewBackForwardNavigationGestures(.disabled)
                // The page never pans sideways (body clips at the viewport,
                // plan 42), so a diagonal drag must not rubber-band it either:
                // horizontal bounce only when the content is actually wider,
                // as MBR does. Vertical keeps the usual bounce.
                .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
                // …and a swipe in from the left edge is that same back.
                .gesture(EdgeSwipe(edge: .left, isEnabled: true) {
                    Haptics.play(NativeHapticStyle.light.rawValue)
                    self.back()
                })
                // A pinch steps the SPA's UI scale (the page reflows); WebKit's
                // own page zoom is off (NativeBridge's viewport script).
                .gesture(PinchToScale { direction in
                    Haptics.play(NativeHapticStyle.light.rawValue)
                    Task { await NativeBridge.zoom(direction, in: session.page) }
                })
                // Edge to edge: the page scrolls under the home indicator. That
                // zeroes `env(safe-area-inset-bottom)` inside the page, so the
                // inset is measured here and handed over (`--ll-native-bottom`).
                .ignoresSafeArea(edges: .bottom)
                // Read from the window: inside a view that ignores the safe
                // area SwiftUI reports 0 too. Re-read whenever the size
                // changes (rotation, split view).
                .background {
                    GeometryReader { proxy in
                        Color.clear.onChange(of: proxy.size, initial: true) {
                            session.setBottomInset(Self.windowBottomInset)
                        }
                    }
                }
                .safeAreaInset(edge: .top, spacing: 0) {
                    if let problem = session.refreshProblem {
                        RefreshProblemBanner(message: problem) { session.refreshProblem = nil }
                    }
                }
        } else if self.lockedOut {
            ContentUnavailableView {
                Label("\(self.ledgerName) Is Locked", systemImage: AppLock.methodSymbol)
            } description: {
                Text("Unlock with \(AppLock.methodName) to open it.")
            } actions: {
                Button("Unlock") { Task { await self.open() } }
                    .buttonStyle(.borderedProminent)
            }
        } else if let needsFolder {
            ContentUnavailableView {
                Label("This Journal Includes Other Files", systemImage: "folder.badge.questionmark")
            } description: {
                Text(needsFolder)
            } actions: {
                Button("Grant Folder Access…") { self.grantFolder() }
                    .buttonStyle(.borderedProminent)
            }
        } else if let error {
            ContentUnavailableView {
                Label("Could Not Open \(self.ledgerName)", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            } actions: {
                Button("Try Again") { Task { await self.open() } }
                    .buttonStyle(.borderedProminent)
            }
        } else {
            ProgressView(self.downloading ? "Downloading \(self.ledgerName)…" : "Opening \(self.ledgerName)…")
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button("Back", systemImage: "chevron.backward") { self.back() }
                .accessibilityHint("Goes back in the ledger, or to the ledger list")
        }
        if let session {
            // One item, not a `ToolbarItemGroup`: beside a long title an
            // iPhone folds a group into a single overflow menu.
            ToolbarItem(placement: .topBarTrailing) {
                HStack(spacing: 4) {
                    LedgerMenu(session: session, switchLedger: { self.model.showHome() })
                }
                .labelStyle(.iconOnly)
            }
        }
    }

    private var ledger: Ledger? {
        guard case let .ledger(id) = self.route else { return nil }
        return self.store.ledger(id)
    }

    private var ledgerName: String {
        self.ledger?.name ?? "Ledger"
    }

    /// The page's section, else the ledger's name.
    private var pageTitle: String {
        let reported = self.session?.info.title ?? ""
        return reported.isEmpty ? self.ledgerName : reported
    }

    // MARK: Actions

    private func back() {
        guard let session else {
            self.model.showHome()
            return
        }
        Task {
            if await NativeBridge.back(in: session.page) { return }
            self.model.showHome()
        }
    }

    private func open() async {
        guard self.session == nil, let ledger else {
            if self.ledger == nil { self.error = "This ledger is no longer in the list." }
            return
        }
        self.error = nil
        self.needsFolder = nil
        self.downloading = false
        // Face ID / passcode first (plan 42): nothing is read until it passes.
        if ledger.requiresUnlock {
            guard await self.model.lock.authenticate(reason: "Unlock \(ledger.name)") else {
                self.lockedOut = true
                return
            }
        }
        self.lockedOut = false
        // A journal whose files are still in the cloud: their downloads were
        // requested; open again until they land (about two minutes).
        for attempt in 0 ..< 60 {
            do {
                let opened = try await model.open(ledger)
                self.session = opened
                self.downloading = false
                await opened.start()
                return
            } catch let MobileError.NotDownloaded(paths) {
                self.downloading = true
                if attempt == 0 { Log.info("open: waiting for \(paths.count) file(s) to download") }
                try? await Task.sleep(for: .seconds(2))
                if Task.isCancelled { return }
            } catch let MobileError.NeedsFolder(message) {
                self.needsFolder = message
                return
            } catch {
                self.error = Self.describe(error)
                Log.info("open failed: \(error)")
                return
            }
        }
        self.downloading = false
        self.error = "The journal's files have not finished downloading. Try again when they have."
    }

    /// Grant Folder Access: the folder picker, opening beside the journal.
    private func grantFolder() {
        guard let ledger else { return }
        let file = URL(filePath: ledger.location)
        DocumentPicker.pickFolder(start: file.deletingLastPathComponent()) { url in
            guard let url else { return }
            let access = ScopedAccess(url: url, scoped: url.startAccessingSecurityScopedResource())
            let picked = PickedFolder(access: access, replacing: ledger.id)
            if let relative = relativePath(of: file, in: url) {
                Task { await self.becomeFolder(picked, mainFile: relative) }
            } else {
                // Somewhere else: let the user pick the journal inside it.
                self.pickedFolder = picked
            }
        }
    }

    /// The file ledger now opens through `folder`; open it again.
    private func becomeFolder(_ folder: PickedFolder, mainFile: String) async {
        guard await self.store.add(folder: folder.access.url, mainFile: mainFile, replacing: folder.replacing) != nil
        else { return }
        await self.open()
    }

    /// The key window's bottom safe-area inset (the home indicator), pt.
    private static var windowBottomInset: CGFloat {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let window = scenes.flatMap(\.windows).first(where: \.isKeyWindow) ?? scenes.first?.windows.first
        return window?.safeAreaInsets.bottom ?? 0
    }

    static func describe(_ error: Error) -> String {
        switch error {
        case let MobileError.Open(message): message
        case let MobileError.Internal(message): message
        case let MobileError.NotDownloaded(paths): "Not downloaded yet: \(paths.joined(separator: ", "))"
        default: error.localizedDescription
        }
    }
}

/// The hamburger: the page's sections (the active one checked), its actions,
/// then the app's own.
private struct LedgerMenu: View {
    let session: LedgerSession
    let switchLedger: () -> Void

    var body: some View {
        Menu("Menu", systemImage: "line.3.horizontal") {
            Section {
                ForEach(self.session.info.sections) { section in
                    Button {
                        Task {
                            let ok = await NativeBridge.navigate(section.id, in: self.session.page)
                            if !ok { Log.info("bridge: navigate \(section.id) declined") }
                        }
                    } label: {
                        if section.active {
                            Label(section.label, systemImage: "checkmark")
                        } else if let symbol = section.symbol {
                            Label(section.label, systemImage: symbol)
                        } else {
                            Text(section.label)
                        }
                    }
                }
            }
            Section {
                ForEach(self.session.info.actions) { action in
                    Button(role: action.destructive ? .destructive : nil) {
                        Task {
                            let ok = await NativeBridge.action(action.id, in: self.session.page)
                            if !ok { Log.info("bridge: action \(action.id) declined") }
                        }
                    } label: {
                        if let icon = action.icon {
                            Label(action.label, systemImage: icon)
                        } else {
                            Text(action.label)
                        }
                    }
                }
            }
            Section {
                Button("Refresh", systemImage: "arrow.clockwise") {
                    Task { await self.session.refresh(reason: "menu") }
                }
                Button("Switch Ledger", systemImage: "books.vertical", action: self.switchLedger)
            }
        }
    }
}

/// "The journal no longer reads": shown over the page after a refresh that
/// failed; the last good snapshot stays on screen.
private struct RefreshProblemBanner: View {
    let message: String
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
            VStack(alignment: .leading, spacing: 2) {
                Text("Showing the last version that loaded").font(.footnote.bold())
                Text(self.message).font(.caption).lineLimit(4)
            }
            Spacer(minLength: 0)
            Button("Dismiss", systemImage: "xmark", action: self.dismiss)
                .labelStyle(.iconOnly)
                .font(.footnote)
        }
        .padding(10)
        .background(.yellow.opacity(0.15))
        .background(.background)
        .accessibilityElement(children: .combine)
    }
}

/// A swipe in from one screen edge: UIKit's `UIScreenEdgePanGestureRecognizer`
/// (as MBR's app does), so it begins only at the edge and follows the system's
/// own edge rules. Attached to the web view as an extra recognizer, not drawn
/// over it: a touch that does not start at the edge, a tap and a vertical
/// scroll all reach the page untouched.
///
/// It fires once per swipe, as soon as the drag is clearly inward and
/// horizontal (or on a short, fast flick at the end); once it fires, UIKit
/// cancels the page's touches, so a journal card under the finger springs back
/// rather than taking the drag as its own swipe.
private struct EdgeSwipe: UIGestureRecognizerRepresentable {
    /// `.left` or `.right`.
    let edge: UIRectEdge
    let isEnabled: Bool
    let action: () -> Void

    /// Inward distance, in points, at which a drag counts.
    private static let distance: CGFloat = 40
    /// A flick: this inward speed (pt/s) at the end, over at least `flickDistance`.
    private static let flickSpeed: CGFloat = 400
    private static let flickDistance: CGFloat = 16

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var fired = false

        /// Alongside the web view's own recognizers, so it never waits on
        /// (or blocks) scrolling and selection.
        func gestureRecognizer(
            _: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith _: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }

    func makeCoordinator(converter _: CoordinateSpaceConverter) -> Coordinator {
        Coordinator()
    }

    func makeUIGestureRecognizer(context: Context) -> UIScreenEdgePanGestureRecognizer {
        let recognizer = UIScreenEdgePanGestureRecognizer()
        recognizer.edges = self.edge
        recognizer.delegate = context.coordinator
        recognizer.isEnabled = self.isEnabled
        return recognizer
    }

    func updateUIGestureRecognizer(_ recognizer: UIScreenEdgePanGestureRecognizer, context _: Context) {
        recognizer.isEnabled = self.isEnabled
    }

    func handleUIGestureRecognizerAction(_ recognizer: UIScreenEdgePanGestureRecognizer, context: Context) {
        let view = recognizer.view
        // Inward is +x from the left edge, -x from the right.
        let sign: CGFloat = self.edge == .right ? -1 : 1
        let inward = recognizer.translation(in: view).x * sign
        let across = abs(recognizer.translation(in: view).y)
        let horizontal = inward > across * 1.5
        switch recognizer.state {
        case .began:
            context.coordinator.fired = false
        case .changed:
            if !context.coordinator.fired, horizontal, inward >= Self.distance {
                context.coordinator.fired = true
                self.action()
            }
        case .ended:
            let speed = recognizer.velocity(in: view).x * sign
            if !context.coordinator.fired, horizontal, inward >= Self.flickDistance, speed >= Self.flickSpeed {
                context.coordinator.fired = true
                self.action()
            }
        default:
            break
        }
    }
}

/// A pinch steps the SPA's interface scale (plan 42): one step in or out each
/// time the fingers spread or close by `PinchSteps.ratio` since the last step,
/// so a long pinch walks several steps and the page reflows at each rather
/// than magnifying. Recognized alongside the web view's own gestures.
private struct PinchToScale: UIGestureRecognizerRepresentable {
    let step: (PinchSteps.Direction) -> Void

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var steps = PinchSteps()

        func gestureRecognizer(
            _: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith _: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }

    func makeCoordinator(converter _: CoordinateSpaceConverter) -> Coordinator {
        Coordinator()
    }

    func makeUIGestureRecognizer(context: Context) -> UIPinchGestureRecognizer {
        let recognizer = UIPinchGestureRecognizer()
        recognizer.delegate = context.coordinator
        return recognizer
    }

    func handleUIGestureRecognizerAction(_ recognizer: UIPinchGestureRecognizer, context: Context) {
        switch recognizer.state {
        case .began:
            context.coordinator.steps = PinchSteps()
        case .changed:
            for direction in context.coordinator.steps.advance(to: recognizer.scale) {
                self.step(direction)
            }
        default:
            break
        }
    }
}
