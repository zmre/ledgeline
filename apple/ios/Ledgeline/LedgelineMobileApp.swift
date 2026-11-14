import SwiftUI

/// Ledgeline for iOS and iPadOS (plans/42-mobile-app.md). Named so as not to
/// clash with the Rust core's `LedgelineApp`.
@main
struct LedgelineMobileApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(self.model)
                .environment(self.model.store)
                .onOpenURL { self.model.openDocument($0) }
        }
    }
}
