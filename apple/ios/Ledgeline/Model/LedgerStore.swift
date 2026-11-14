import Foundation
import Observation

/// A saved ledger: a journal file the user granted, a folder plus the journal
/// inside it, or the bundled sample.
struct Ledger: Identifiable, Codable, Hashable {
    enum Scope: String, Codable {
        /// `bookmark` is the folder; `mainFile` is relative to it.
        case folder
        /// `bookmark` is the journal file itself (`mainFile` is its name).
        /// Its `include`s are out of reach: opening one that has any asks
        /// for the folder.
        case file
        /// The bundled sample journal, copied into Documents/Sample.
        case sample
    }

    var id: UUID
    /// Shown in the list and as the bar's subtitle. Starts as the journal's
    /// name; the user can rename it.
    var name: String
    var scope: Scope
    /// The journal, relative to the folder (`scope == .folder`), or its file
    /// name.
    var mainFile: String
    /// Plain `bookmarkData()` of the granted URL. (On iOS a plain bookmark of
    /// a security-scoped URL carries the grant.) `nil` for the sample.
    var bookmark: Data?
    /// Where it was when added, for the list's second line. Display only.
    var location: String
}

/// Where the app keeps things.
enum AppPaths {
    /// The sample journal, under Documents so it is writable and shows up in
    /// Files (UIFileSharingEnabled).
    static var sampleFolder: URL {
        URL.documentsDirectory.appending(path: "Sample", directoryHint: .isDirectory)
    }

    static let sampleFile = "sample.journal"

    /// Per-ledger scratch (dropped statements' staging area).
    static func cacheFolder(for id: UUID) -> URL {
        URL.cachesDirectory.appending(path: "ledgers/\(id.uuidString)", directoryHint: .isDirectory)
    }

    /// App-wide settings (`ui-prefs.toml`), shared by every ledger.
    static var configFolder: URL {
        URL.applicationSupportDirectory.appending(path: "Ledgeline", directoryHint: .isDirectory)
    }
}

/// Security-scoped access to a ledger's folder (or file), held for as long as
/// the `LedgelineApp` reading it lives, and released when this goes.
final class ScopedAccess: @unchecked Sendable {
    let url: URL
    private let scoped: Bool

    init(url: URL, scoped: Bool) {
        self.url = url
        self.scoped = scoped
    }

    deinit {
        if scoped { url.stopAccessingSecurityScopedResource() }
    }

    var path: String {
        self.url.path(percentEncoded: false)
    }
}

enum LedgerAccessError: LocalizedError {
    case accessDenied(String)
    case bookmarkUnresolvable(String)
    case notInside(file: String, folder: String)

    var errorDescription: String? {
        switch self {
        case let .accessDenied(path):
            "iOS no longer lets Ledgeline read \(path). Add it again to restore access."
        case let .bookmarkUnresolvable(reason):
            "Ledgeline could not find this ledger any more (\(reason)). Add it again to restore access."
        case let .notInside(file, folder):
            "\(file) is not inside \(folder). Pick the folder that contains the journal."
        }
    }
}

/// The saved ledgers, persisted in `UserDefaults` as JSON.
@MainActor
@Observable
final class LedgerStore {
    private(set) var ledgers: [Ledger] = []
    var lastError: String?

    private let defaultsKey = "ledgeline.ledgers.v1"
    private let sampleSeededKey = "ledgeline.sampleSeeded"

    init() {
        self.load()
        // Seed the sample once, so a first launch has something to open; a
        // removed sample stays removed (Add › Sample Ledger brings it back).
        if !UserDefaults.standard.bool(forKey: self.sampleSeededKey) {
            UserDefaults.standard.set(true, forKey: self.sampleSeededKey)
            if !self.hasSample { self.addSample() }
        }
    }

    func ledger(_ id: UUID) -> Ledger? {
        self.ledgers.first { $0.id == id }
    }

    var hasSample: Bool {
        self.ledgers.contains { $0.scope == .sample }
    }

    // MARK: Changes

    func addSample() {
        guard !self.hasSample else { return }
        self.ledgers.insert(
            Ledger(
                id: UUID(), name: "Sample ledger", scope: .sample, mainFile: AppPaths.sampleFile,
                bookmark: nil, location: "Ledgeline › Sample › sample.journal"
            ),
            at: 0
        )
        self.save()
    }

    /// Remembers a journal file the picker (or Files) handed over. A file
    /// already in the list is not added twice.
    @discardableResult
    func add(file url: URL) async -> Ledger? {
        let path = url.standardizedFileURL.path(percentEncoded: false)
        if let existing = ledgers.first(where: { $0.scope == .file && $0.location == path }) {
            return existing
        }
        do {
            let bookmark = try await Self.makeBookmark(url)
            let ledger = Ledger(
                id: UUID(), name: Self.displayName(url.lastPathComponent), scope: .file,
                mainFile: url.lastPathComponent, bookmark: bookmark, location: path
            )
            self.ledgers.append(ledger)
            self.save()
            Log.info("ledger added: file \(path)")
            return ledger
        } catch {
            self.lastError = "Could not remember that file: \(error.localizedDescription)"
            return nil
        }
    }

    /// Remembers a folder and the journal inside it (`mainFile`, relative).
    /// With `replacing`, the existing ledger becomes this one, keeping its id
    /// and name (Grant Folder Access for a file whose journal has includes).
    @discardableResult
    func add(folder url: URL, mainFile: String, replacing id: UUID? = nil) async -> Ledger? {
        do {
            let bookmark = try await Self.makeBookmark(url)
            let location = url.appending(path: mainFile).standardizedFileURL.path(percentEncoded: false)
            if let id, let index = ledgers.firstIndex(where: { $0.id == id }) {
                self.ledgers[index].scope = .folder
                self.ledgers[index].mainFile = mainFile
                self.ledgers[index].bookmark = bookmark
                self.ledgers[index].location = location
                self.save()
                Log.info("ledger \(id) now opens through its folder \(url.path(percentEncoded: false))")
                return self.ledgers[index]
            }
            let ledger = Ledger(
                id: UUID(), name: Self.displayName((mainFile as NSString).lastPathComponent), scope: .folder,
                mainFile: mainFile, bookmark: bookmark, location: location
            )
            self.ledgers.append(ledger)
            self.save()
            Log.info("ledger added: folder \(url.path(percentEncoded: false)), main \(mainFile)")
            return ledger
        } catch {
            self.lastError = "Could not remember that folder: \(error.localizedDescription)"
            return nil
        }
    }

    func rename(_ id: UUID, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let index = ledgers.firstIndex(where: { $0.id == id }) else { return }
        self.ledgers[index].name = trimmed
        self.save()
    }

    /// Forgets a ledger and deletes its scratch folder. The journal itself is
    /// never touched (nor is the sample's copy in Documents).
    func remove(_ id: UUID) {
        self.ledgers.removeAll { $0.id == id }
        self.save()
        let cache = AppPaths.cacheFolder(for: id)
        Task.detached(priority: .utility) {
            try? FileManager.default.removeItem(at: cache)
        }
    }

    // MARK: Opening

    /// Resolves a ledger's bookmark and starts security-scoped access. Off
    /// the main thread: resolving can wait on the file provider. A stale
    /// bookmark is re-saved.
    func access(_ ledger: Ledger) async throws -> ScopedAccess {
        guard let bookmark = ledger.bookmark else {
            let folder = AppPaths.sampleFolder
            try await Self.installSample(in: folder)
            return ScopedAccess(url: folder, scoped: false)
        }
        let (access, fresh) = try await Task.detached(priority: .userInitiated) {
            try Self.resolve(bookmark)
        }.value
        if let fresh, let index = ledgers.firstIndex(where: { $0.id == ledger.id }) {
            Log.info("bookmark stale for ledger \(ledger.id); refreshed")
            self.ledgers[index].bookmark = fresh
            self.save()
        }
        return access
    }

    nonisolated static func resolve(_ bookmark: Data) throws -> (ScopedAccess, Data?) {
        var isStale = false
        let url: URL
        do {
            url = try URL(resolvingBookmarkData: bookmark, options: [], relativeTo: nil, bookmarkDataIsStale: &isStale)
        } catch {
            throw LedgerAccessError.bookmarkUnresolvable(error.localizedDescription)
        }
        let scoped = url.startAccessingSecurityScopedResource()
        let access = ScopedAccess(url: url, scoped: scoped)
        guard FileManager.default.isReadableFile(atPath: access.path) else {
            throw LedgerAccessError.accessDenied(access.path)
        }
        let fresh = isStale
            ? try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
            : nil
        return (access, fresh)
    }

    nonisolated static func makeBookmark(_ url: URL) async throws -> Data {
        try await Task.detached(priority: .userInitiated) {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            return try url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
        }.value
    }

    /// Copies the bundled sample journal into `folder` the first time it is
    /// opened; after that it is the user's to edit.
    nonisolated static func installSample(in folder: URL) async throws {
        try await Task.detached(priority: .userInitiated) {
            let fileManager = FileManager.default
            let target = folder.appending(path: AppPaths.sampleFile)
            guard !fileManager.fileExists(atPath: target.path(percentEncoded: false)) else { return }
            guard let bundled = Bundle.main.url(forResource: "sample", withExtension: "journal") else {
                throw CocoaError(.fileNoSuchFile)
            }
            try fileManager.createDirectory(at: folder, withIntermediateDirectories: true)
            try fileManager.copyItem(at: bundled, to: target)
            Log.info("sample journal installed at \(target.path(percentEncoded: false))")
        }.value
    }

    /// `2026.journal` → `2026`.
    nonisolated static func displayName(_ fileName: String) -> String {
        let stem = (fileName as NSString).deletingPathExtension
        return stem.isEmpty ? fileName : stem
    }

    // MARK: Persistence

    private func load() {
        guard let data = UserDefaults.standard.data(forKey: defaultsKey) else { return }
        do {
            self.ledgers = try JSONDecoder().decode([Ledger].self, from: data)
        } catch {
            Log.info("ledger list unreadable, starting empty: \(error)")
        }
    }

    private func save() {
        do {
            try UserDefaults.standard.set(JSONEncoder().encode(self.ledgers), forKey: self.defaultsKey)
        } catch {
            Log.info("ledger list not saved: \(error)")
        }
    }
}
