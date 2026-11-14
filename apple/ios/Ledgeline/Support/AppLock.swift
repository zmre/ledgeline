import Foundation
import LocalAuthentication
import Observation

/// Face ID / Touch ID / passcode in front of a ledger (plan 42 phase 3).
///
/// A ledger that requires it (`Ledger.requiresUnlock`, on by default) asks
/// when it is opened, and again when the app comes back after more than
/// `relockAfter` in the background; until then the privacy cover stays up.
/// `.deviceOwnerAuthentication` falls back to the passcode by itself. A device
/// with neither (a simulator with nothing enrolled) opens unlocked, and says
/// so in the log.
@MainActor
@Observable
final class AppLock {
    /// How long the app may sit in the background before the ledger locks again.
    static let relockAfter: TimeInterval = 5 * 60

    /// The open ledger is covered until the owner authenticates.
    private(set) var locked = false
    private var backgroundedAt: Date?
    private var authenticating = false

    /// What unlocks on this device: "Face ID", "Touch ID", "Optic ID", else "Passcode".
    static var methodName: String {
        let context = LAContext()
        var error: NSError?
        _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        switch context.biometryType {
        case .faceID: return "Face ID"
        case .touchID: return "Touch ID"
        case .opticID: return "Optic ID"
        default: return "Passcode"
        }
    }

    /// The SF Symbol for `methodName`.
    static var methodSymbol: String {
        switch self.methodName {
        case "Face ID": "faceid"
        case "Touch ID": "touchid"
        case "Optic ID": "opticid"
        default: "lock"
        }
    }

    /// Asks the owner. True when they authenticated, or when the device has
    /// no way to ask (logged); false when they cancelled or failed.
    func authenticate(reason: String) async -> Bool {
        let context = LAContext()
        var error: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) else {
            Log.info("lock: no biometrics or passcode set up (LAError \(error?.code ?? 0)); opening unlocked")
            return true
        }
        do {
            return try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
        } catch {
            Log.info("lock: not authenticated (LAError \((error as NSError).code))")
            return false
        }
    }

    // MARK: Background and foreground

    func didEnterBackground() {
        self.backgroundedAt = .now
    }

    /// Back in the foreground: lock when the app was away for longer than
    /// `relockAfter` and the open ledger asks for it. The cover stays up
    /// until `unlock` succeeds.
    func didBecomeActive(requiresUnlock: Bool) {
        defer { self.backgroundedAt = nil }
        guard requiresUnlock, let since = backgroundedAt,
              Date.now.timeIntervalSince(since) > Self.relockAfter
        else { return }
        Log.info("lock: away for more than \(Int(Self.relockAfter))s; locking")
        self.locked = true
    }

    /// Asks the owner to lift the cover. Ignored while a prompt is already up.
    func unlock(reason: String) async {
        guard self.locked, !self.authenticating else { return }
        self.authenticating = true
        defer { self.authenticating = false }
        if await self.authenticate(reason: reason) {
            self.locked = false
        }
    }

    /// No ledger is open any more (back at the list): nothing to cover.
    func clear() {
        self.locked = false
    }
}
