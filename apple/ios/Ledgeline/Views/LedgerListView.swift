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
        // cover the figures before it is. A locked ledger (Face ID, plan 42)
        // keeps the cover, with a way to unlock, until the owner does.
        .overlay {
            if self.scenePhase != .active || self.model.lock.locked {
                PrivacyCover(
                    locked: self.model.lock.locked,
                    unlock: { self.model.unlock() },
                    leave: { self.model.showHome() }
                )
                .transition(.opacity)
            }
        }
        .onChange(of: self.scenePhase) { _, phase in
            switch phase {
            case .active: self.model.didBecomeActive()
            case .background: self.model.didEnterBackground()
            default: break
            }
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
/// ledger, and what a locked ledger shows until the owner unlocks it.
struct PrivacyCover: View {
    var locked = false
    var unlock: () -> Void = {}
    var leave: () -> Void = {}

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
                if self.locked {
                    Button("Unlock with \(AppLock.methodName)", systemImage: AppLock.methodSymbol, action: self.unlock)
                        .buttonStyle(.borderedProminent)
                        .padding(.top, 8)
                    Button("Ledgers", action: self.leave)
                }
            }
        }
        .ignoresSafeArea()
        .accessibilityHidden(!self.locked)
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
                        Toggle(isOn: self.requireUnlockBinding(ledger)) {
                            Label("Require \(AppLock.methodName)", systemImage: AppLock.methodSymbol)
                        }
                        Button("Remove", systemImage: "trash", role: .destructive) { self.store.remove(ledger.id) }
                    }
                    .swipeActions {
                        Button("Remove", systemImage: "trash", role: .destructive) { self.store.remove(ledger.id) }
                        Button("Rename", systemImage: "pencil") { self.beginRename(ledger) }
                            .tint(.indigo)
                    }
                    .swipeActions(edge: .leading) {
                        Button(
                            ledger.requiresUnlock ? "No \(AppLock.methodName)" : "Require \(AppLock.methodName)",
                            systemImage: ledger.requiresUnlock ? "lock.open" : "lock"
                        ) { self.setRequiresUnlock(ledger, !ledger.requiresUnlock) }
                            .tint(.teal)
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

    private func requireUnlockBinding(_ ledger: Ledger) -> Binding<Bool> {
        Binding(
            get: { self.store.ledger(ledger.id)?.requiresUnlock ?? true },
            set: { self.setRequiresUnlock(ledger, $0) }
        )
    }

    /// Turning the lock ON needs nothing; turning it OFF asks the owner first,
    /// or anyone holding the unlocked phone could switch it off.
    private func setRequiresUnlock(_ ledger: Ledger, _ required: Bool) {
        if required {
            self.store.setRequiresUnlock(ledger.id, true)
            return
        }
        Task {
            guard await self.model.lock.authenticate(reason: "Stop asking before opening \(ledger.name)") else { return }
            self.store.setRequiresUnlock(ledger.id, false)
        }
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
                HStack(spacing: 4) {
                    Text(self.ledger.name)
                    if self.ledger.requiresUnlock {
                        Image(systemName: "lock.fill")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .accessibilityLabel("Locked with \(AppLock.methodName)")
                    }
                }
                Text(self.ledger.scope == .sample
                    ? "Example data, fresh and dated to this month each time it opens"
                    : Self.prettyLocation(self.ledger.location))
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
