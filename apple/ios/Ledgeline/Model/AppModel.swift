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
    let lock = AppLock()

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
        self.lock.clear()
    }

    /// The ledger on screen asks for Face ID / passcode (plan 42).
    var sessionRequiresUnlock: Bool {
        guard let session else { return false }
        return self.store.ledger(session.ledgerID)?.requiresUnlock ?? false
    }

    /// The on-screen ledger's name, for the unlock prompt.
    var sessionLedgerName: String {
        self.session.flatMap { self.store.ledger($0.ledgerID)?.name } ?? "Ledgeline"
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
        self.lock.didBecomeActive(requiresUnlock: self.sessionRequiresUnlock)
        if self.lock.locked { self.unlock() }
        guard let session else { return }
        Task { await session.refresh(reason: "foreground") }
    }

    /// Off to the background: the relock clock starts.
    func didEnterBackground() {
        self.lock.didEnterBackground()
    }

    /// Ask the owner to lift the lock cover.
    func unlock() {
        let name = self.sessionLedgerName
        Task { await self.lock.unlock(reason: "Unlock \(name)") }
    }
}
