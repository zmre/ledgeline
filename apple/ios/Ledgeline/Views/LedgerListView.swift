import SwiftUI
import UIKit

/// The app's root: the ledger list, with the open ledger pushed on top, and
/// the privacy cover whenever the scene is not active.
struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        @Bindable var model = model
        NavigationStack(path: $model.path) {
            LedgerListView()
                // Keyed by the route: SwiftUI would otherwise reuse the screen
                // at that position, with the old ledger's session in its state.
                .navigationDestination(for: Route.self) { route in
                    LedgerScreen(route: route).id(route)
                }
        }
        // The app switcher's snapshot is taken while the scene is inactive:
        // cover the figures before it is.
        .overlay {
            if self.scenePhase != .active {
                PrivacyCover().transition(.opacity)
            }
        }
        .onChange(of: self.scenePhase) { _, phase in
            if phase == .active { self.model.didBecomeActive() }
        }
        #if DEBUG
        // Screenshots without a tap: `SIMCTL_CHILD_LEDGELINE_OPEN_SAMPLE=1
        // xcrun simctl launch …` opens the sample ledger at launch.
        .task {
            guard ProcessInfo.processInfo.environment["LEDGELINE_OPEN_SAMPLE"] == "1" else { return }
            self.model.store.addSample()
            if let sample = self.model.store.ledgers.first(where: { $0.scope == .sample }) {
                self.model.path = [.ledger(sample.id)]
            }
        }
        #endif
    }
}

/// Opaque, with the app's icon: what the app switcher shows instead of the
/// ledger.
struct PrivacyCover: View {
    var body: some View {
        ZStack {
            Color(uiColor: .systemBackground)
            VStack(spacing: 16) {
                if let icon = UIImage(named: "ledgeline.png") {
                    Image(uiImage: icon)
                        .resizable()
                        .frame(width: 96, height: 96)
                        .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
                }
                Text("Ledgeline").font(.title2.weight(.semibold))
            }
        }
        .ignoresSafeArea()
        .accessibilityHidden(true)
    }
}

/// The home screen: saved ledgers, add / rename / remove.
struct LedgerListView: View {
    @Environment(AppModel.self) private var model
    @Environment(LedgerStore.self) private var store
    @State private var renaming: Ledger?
    @State private var newName = ""
    @State private var pickedFolder: PickedFolder?

    var body: some View {
        List {
            Section {
                ForEach(self.store.ledgers) { ledger in
                    NavigationLink(value: Route.ledger(ledger.id)) {
                        LedgerRow(ledger: ledger)
                    }
                    .contextMenu {
                        Button("Rename", systemImage: "pencil") { self.beginRename(ledger) }
                        Button("Remove", systemImage: "trash", role: .destructive) { self.store.remove(ledger.id) }
                    }
                    .swipeActions {
                        Button("Remove", systemImage: "trash", role: .destructive) { self.store.remove(ledger.id) }
                        Button("Rename", systemImage: "pencil") { self.beginRename(ledger) }
                            .tint(.indigo)
                    }
                }
            } footer: {
                Text("Add a journal with +. Ledgeline reads and edits it where it is; nothing is copied. "
                    + "Removing a ledger here never deletes its files.")
            }
            if let error = store.lastError {
                Section("Last error") {
                    Text(error).foregroundStyle(.red)
                    Button("Dismiss") { self.store.lastError = nil }
                }
            }
        }
        .overlay {
            if self.store.ledgers.isEmpty {
                ContentUnavailableView {
                    Label("No Ledgers", systemImage: "book.closed")
                } description: {
                    Text("Open an hledger journal from Files or iCloud Drive.")
                } actions: {
                    self.addMenu
                }
            }
        }
        .navigationTitle("Ledgers")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                self.addMenu
            }
        }
        .sheet(item: self.$pickedFolder) { folder in
            JournalChooser(folder: folder) { mainFile in
                Task {
                    if let ledger = await self.store.add(
                        folder: folder.access.url, mainFile: mainFile, replacing: folder.replacing
                    ) {
                        self.model.path = [.ledger(ledger.id)]
                    }
                }
            }
        }
        .alert("Rename Ledger", isPresented: self.renamingBinding) {
            TextField("Name", text: self.$newName)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                if let renaming { self.store.rename(renaming.id, to: self.newName) }
            }
        } message: {
            Text("Only the name shown in Ledgeline changes; the journal keeps its file name.")
        }
    }

    private var addMenu: some View {
        Menu("Add Ledger", systemImage: "plus") {
            Button("Open Journal File…", systemImage: "doc.text") { self.addFile() }
            Button("Open Folder…", systemImage: "folder") { self.addFolder() }
            if !self.store.hasSample {
                Divider()
                Button("Add Sample Ledger", systemImage: "book.closed") { self.store.addSample() }
            }
        }
    }

    private var renamingBinding: Binding<Bool> {
        Binding(get: { self.renaming != nil }, set: { if !$0 { self.renaming = nil } })
    }

    private func beginRename(_ ledger: Ledger) {
        self.newName = ledger.name
        self.renaming = ledger
    }

    private func addFile() {
        DocumentPicker.pickJournal { url in
            guard let url else { return }
            Task {
                if let ledger = await self.store.add(file: url) {
                    self.model.path = [.ledger(ledger.id)]
                }
            }
        }
    }

    private func addFolder() {
        DocumentPicker.pickFolder { url in
            guard let url else { return }
            let access = ScopedAccess(url: url, scoped: url.startAccessingSecurityScopedResource())
            self.pickedFolder = PickedFolder(access: access, replacing: nil)
        }
    }
}

/// One saved ledger: its name, and where its journal is.
private struct LedgerRow: View {
    let ledger: Ledger

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(self.ledger.name)
                Text(Self.prettyLocation(self.ledger.location))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .truncationMode(.middle)
            }
        } icon: {
            Image(systemName: self.symbol)
        }
    }

    /// The stored path, with the system's long container prefixes named the
    /// way Files names them.
    static func prettyLocation(_ path: String) -> String {
        let prefixes = [
            ("/private/var/mobile/Library/Mobile Documents/com~apple~CloudDocs", "iCloud Drive"),
            ("/var/mobile/Library/Mobile Documents/com~apple~CloudDocs", "iCloud Drive"),
        ]
        for (prefix, name) in prefixes where path.hasPrefix(prefix) {
            return name + path.dropFirst(prefix.count)
        }
        return path
    }

    private var symbol: String {
        switch self.ledger.scope {
        case .sample: "book.closed"
        case .file: "doc.text"
        case .folder: "folder"
        }
    }
}
