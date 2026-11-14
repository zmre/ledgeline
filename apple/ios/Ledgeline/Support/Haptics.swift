import UIKit

/// The page's `haptic` message (`{style: "light" | "success" | "warning"}`).
@MainActor
enum Haptics {
    static func play(_ style: String) {
        switch NativeHapticStyle(rawValue: style) {
        case .light:
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        case .success:
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        case .warning:
            UINotificationFeedbackGenerator().notificationOccurred(.warning)
        case nil:
            Log.info("haptic: unknown style \(style)")
        }
    }
}
