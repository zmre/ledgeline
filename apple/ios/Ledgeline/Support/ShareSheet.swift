import UIKit

/// The page's `share` message: an export (xlsx, …) handed to the system share
/// sheet instead of an `<a download>`, so Save to Files, AirDrop and Mail work.
@MainActor
enum ShareSheet {
    /// Writes `base64` to a temporary file named `filename` and shares it.
    /// The file is deleted when the sheet goes away.
    static func present(filename: String, mime: String, base64: String) {
        guard let data = Data(base64Encoded: base64, options: .ignoreUnknownCharacters) else {
            Log.info("share: \(filename) is not valid base64; ignored")
            return
        }
        guard let host = Presenter.topViewController() else {
            Log.info("share: no view controller to present from")
            return
        }
        let folder = FileManager.default.temporaryDirectory
            .appending(path: "share-\(UUID().uuidString)", directoryHint: .isDirectory)
        let file = folder.appending(path: self.safeName(filename))
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try data.write(to: file, options: .completeFileProtection)
        } catch {
            Log.info("share: could not stage \(file.lastPathComponent): \(error.localizedDescription)")
            return
        }
        Log.info("share: \(file.lastPathComponent) (\(mime), \(data.count)B)")

        let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        sheet.completionWithItemsHandler = { _, _, _, _ in
            try? FileManager.default.removeItem(at: folder)
        }
        // iPad: a popover needs an anchor. The hamburger sits at the top
        // trailing corner of the bar, so the arrow points there.
        if let popover = sheet.popoverPresentationController, let view = host.view {
            popover.sourceView = view
            let top = view.safeAreaInsets.top
            popover.sourceRect = CGRect(x: view.bounds.maxX - 40, y: max(top - 8, 0), width: 1, height: 1)
            popover.permittedArrowDirections = [.up]
        }
        host.present(sheet, animated: true)
    }

    /// The last path component only, never empty, never hidden: the name
    /// comes from the page.
    static func safeName(_ filename: String) -> String {
        let last = (filename as NSString).lastPathComponent
            .replacingOccurrences(of: ":", with: "-")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmed = last.drop { $0 == "." }
        return trimmed.isEmpty ? "export" : String(trimmed)
    }
}
