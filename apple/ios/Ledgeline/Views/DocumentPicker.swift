import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The system document picker, for a journal file or a folder.
///
/// `UIDocumentPickerViewController` presented from UIKit rather than
/// `.fileImporter` or a representable in a `.sheet`: mbr found the picker's
/// own `directoryURL` is the only reliable way to choose where it opens, and
/// that a representable inside a sheet ignored it. `asCopy: false`: the
/// journal is used in place, never copied.
@MainActor
enum DocumentPicker {
    /// The journal types (assets/Info.plist.in's claims, also declared in
    /// project.yml), plus plain text for journals under other extensions.
    static var journalTypes: [UTType] {
        let declared = ["com.zmre.ledgeline.journal", "com.zmre.ledgeline.ledger", "org.ledger-cli.journal"]
            .compactMap { UTType($0) }
        return declared + [.plainText]
    }

    /// The extensions offered when choosing the journal inside a folder.
    nonisolated static let journalExtensions: Set<String> = ["journal", "hledger", "ledger", "j"]

    /// The delegate of the picker on screen; UIKit holds delegates weakly.
    private static var current: Coordinator?

    static func pickJournal(start: URL? = nil, done: @escaping (URL?) -> Void) {
        self.present(types: self.journalTypes, start: start, done: done)
    }

    static func pickFolder(start: URL? = nil, done: @escaping (URL?) -> Void) {
        self.present(types: [.folder], start: start, done: done)
    }

    private static func present(types: [UTType], start: URL?, done: @escaping (URL?) -> Void) {
        guard let host = Presenter.topViewController() else {
            Log.info("picker: no view controller to present from")
            done(nil)
            return
        }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: types, asCopy: false)
        picker.allowsMultipleSelection = false
        picker.directoryURL = start
        let coordinator = Coordinator { url in
            self.current = nil
            done(url)
        }
        self.current = coordinator
        picker.delegate = coordinator
        host.present(picker, animated: true)
    }

    private final class Coordinator: NSObject, UIDocumentPickerDelegate {
        private let done: (URL?) -> Void

        init(done: @escaping (URL?) -> Void) {
            self.done = done
        }

        func documentPicker(_: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
            self.done(urls.first)
        }

        func documentPickerWasCancelled(_: UIDocumentPickerViewController) {
            self.done(nil)
        }
    }
}

/// `file`'s path relative to `folder`, or `nil` when it is not inside.
func relativePath(of file: URL, in folder: URL) -> String? {
    let filePath = file.resolvingSymlinksInPath().standardizedFileURL.pathComponents
    let folderPath = folder.resolvingSymlinksInPath().standardizedFileURL.pathComponents
    guard filePath.count > folderPath.count, Array(filePath.prefix(folderPath.count)) == folderPath else {
        return nil
    }
    return filePath.dropFirst(folderPath.count).joined(separator: "/")
}

/// A folder the picker returned, and why: a new ledger, or the folder for an
/// existing file ledger whose journal has includes.
struct PickedFolder: Identifiable {
    let id = UUID()
    let access: ScopedAccess
    /// The ledger that becomes this folder's, keeping its id and name.
    let replacing: UUID?
}

/// After a folder is picked: which journal in it is the main one. Lists the
/// journal files up to three levels down; "Choose Another File…" opens the
/// picker inside the folder for anything else.
struct JournalChooser: View {
    let folder: PickedFolder
    let chosen: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var candidates: [String]?
    @State private var error: String?

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if let candidates {
                        if candidates.isEmpty {
                            Text("No .journal, .hledger, .ledger or .j files in this folder.")
                                .foregroundStyle(.secondary)
                        }
                        ForEach(candidates, id: \.self) { path in
                            Button {
                                self.chosen(path)
                                self.dismiss()
                            } label: {
                                Label(path, systemImage: "doc.text")
                            }
                        }
                    } else {
                        ProgressView()
                    }
                } header: {
                    Text(self.folder.access.url.lastPathComponent)
                } footer: {
                    Text("Pick the main journal: the one that includes the others.")
                }
                Section {
                    Button("Choose Another File…", systemImage: "folder") { self.pickInside() }
                }
                if let error {
                    Section { Text(error).foregroundStyle(.red) }
                }
            }
            .navigationTitle("Main Journal")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", role: .cancel) { self.dismiss() }
                }
            }
            .task { self.candidates = await Self.scan(self.folder.access.url) }
        }
    }

    private func pickInside() {
        let root = self.folder.access.url
        DocumentPicker.pickJournal(start: root) { file in
            guard let file else { return }
            if let relative = relativePath(of: file, in: root) {
                self.chosen(relative)
                self.dismiss()
            } else {
                self.error = LedgerAccessError.notInside(
                    file: file.lastPathComponent, folder: root.lastPathComponent
                ).localizedDescription
            }
        }
    }

    /// Journal files under `folder`, shallowest first. Names only: nothing is
    /// read, so nothing in the cloud is downloaded.
    nonisolated static func scan(_ folder: URL) async -> [String] {
        await Task.detached(priority: .userInitiated) {
            var found: [String] = []
            let enumerator = FileManager.default.enumerator(
                at: folder,
                includingPropertiesForKeys: [.isRegularFileKey],
                options: [.skipsHiddenFiles, .skipsPackageDescendants]
            )
            while let url = enumerator?.nextObject() as? URL {
                guard let relative = relativePath(of: url, in: folder) else { continue }
                let depth = relative.split(separator: "/").count
                if depth > 3 {
                    enumerator?.skipDescendants()
                    continue
                }
                if DocumentPicker.journalExtensions.contains(url.pathExtension.lowercased()) {
                    found.append(relative)
                }
                if found.count >= 200 { break }
            }
            return found.sorted {
                let (lhs, rhs) = ($0.split(separator: "/").count, $1.split(separator: "/").count)
                return lhs == rhs ? $0.localizedStandardCompare($1) == .orderedAscending : lhs < rhs
            }
        }.value
    }
}
