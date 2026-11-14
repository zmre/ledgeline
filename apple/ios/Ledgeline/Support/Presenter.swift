import UIKit

/// The view controller UIKit sheets (document picker, share sheet) are
/// presented from: the active scene's topmost one. Presenting from UIKit
/// rather than wrapping the controllers in SwiftUI sheets is what mbr settled
/// on (a representable picker inside a `.sheet` ignored `directoryURL`).
@MainActor
enum Presenter {
    static func topViewController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let scene = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first
        var top = scene?.keyWindow?.rootViewController
        while let presented = top?.presentedViewController, !presented.isBeingDismissed {
            top = presented
        }
        return top
    }
}
