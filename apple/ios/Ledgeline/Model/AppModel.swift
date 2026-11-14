import Foundation
import Observation

/// A screen on the navigation stack.
enum Route: Hashable {
    case ledger(UUID)
}

/// App-wide state: the ledger list, navigation, and the open session.
@MainActor
@Observable
final class AppModel {
    let store = LedgerStore()
    let coordinator = FileCoordinatorBridge()

    var path: [Route] = []
    /// The session on screen. One at a time: opening another closes it.
    private(set) var session: LedgerSession?

    init() {
        #if DEBUG
            assert(FNV1a.selfCheck(), "FNV-1a 64 does not match the README's test vectors")
        #endif
    }

    // MARK: Sessions

    func open(_ ledger: Ledger) async throws -> LedgerSession {
        if let previous = session {
            self.session = nil
            previous.close()
        }
        let opened = try await LedgerSession.open(ledger, store: self.store, coordinator: self.coordinator)
        self.session = opened
        return opened
    }

    func close(_ closing: LedgerSession) {
        guard self.session === closing else { return }
        self.session = nil
        closing.close()
    }

    /// Back to the ledger list.
    func showHome() {
        self.path = []
    }

    /// A journal handed to the app (Files' Open In, a tap on a .journal): saved
    /// to the list (once) and opened, over whatever is on screen.
    func openDocument(_ url: URL) {
        Log.info("document: open \(url.lastPathComponent)")
        Task {
            guard let ledger = await self.store.add(file: url) else { return }
            self.path = [.ledger(ledger.id)]
        }
    }

    // MARK: Lifecycle

    /// Back in the foreground (or the app switcher dismissed): pick up edits
    /// made elsewhere. Cheap when nothing changed.
    func didBecomeActive() {
        guard let session else { return }
        Task { await session.refresh(reason: "foreground") }
    }
}
