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
                .ignoresSafeArea(edges: .bottom)
                .safeAreaInset(edge: .top, spacing: 0) {
                    if let problem = session.refreshProblem {
                        RefreshProblemBanner(message: problem) { session.refreshProblem = nil }
                    }
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
