import Foundation

/// Ledgeline's `FileCoordinator` (crates/ledgeline-ffi/README.md): every
/// write, create, mkdir and delete the Rust core makes in a ledger's folder
/// goes through `NSFileCoordinator`, so the file provider (iCloud Drive,
/// Dropbox, Working Copy, …) sees a coordinated change. Reads never come here;
/// Rust reads with plain `std::fs`.
///
/// Rust calls these from its own background threads and waits for them, so
/// each method blocks until the operation is done. None touches the main
/// thread. Adapted from mbr's `FileCoordinatorBridge`; the difference is the
/// conditional replace, which carries Ledgeline's external-change guarantee
/// across the seam.
final class FileCoordinatorBridge: FileCoordinator, @unchecked Sendable {
    // MARK: FileCoordinator

    /// The conditional replace. Inside one `.forReplacing` coordinated write:
    /// refuse with `Changed` unless `path` is a regular file whose length and
    /// FNV-1a 64 hash are what Rust last read; otherwise replace it atomically
    /// from the item-replacement directory. An edit synced in from another
    /// device between Rust's read and this write is a 409, never a silent
    /// overwrite.
    func coordinatedReplace(path: String, contents: Data, expectedLen: UInt64, expectedHash: String) throws {
        let start = ContinuousClock.now
        var coordinationError: NSError?
        var failure: Error?
        var how = "replace"
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path), options: .forReplacing, error: &coordinationError
        ) { target in
            do {
                try Self.checkUnchanged(target, expectedLen: expectedLen, expectedHash: expectedHash)
            } catch {
                failure = error
                return
            }
            do {
                try Self.replace(target, with: contents)
            } catch {
                // Some providers (SMB shares) refuse atomic replacement. The
                // check above already ran inside this block, so a plain
                // overwrite is still conditional.
                Log.info("coordinator: atomic replace of \(path) failed (\(error.localizedDescription)); "
                    + "writing in place")
                how = "in place"
                do { try contents.write(to: target) } catch { failure = error }
            }
        }
        try Self.check(coordinationError ?? failure, path: path)
        Log.info("coordinator: replace \(path) (\(how)) in \(millis(since: start))ms")
    }

    /// Exclusive create: anything already at `path` is `AlreadyExists`.
    func coordinatedCreate(path: String, contents: Data) throws {
        let start = ContinuousClock.now
        var coordinationError: NSError?
        var failure: Error?
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path), options: [], error: &coordinationError
        ) { target in
            let targetPath = target.path(percentEncoded: false)
            // `attributesOfItem` does not follow symlinks: a dangling one
            // still counts as something being there.
            if (try? FileManager.default.attributesOfItem(atPath: targetPath)) != nil {
                failure = CoordinatorError.AlreadyExists(message: path)
                return
            }
            do {
                // `.withoutOverwriting` is O_EXCL: a racing create loses here.
                try contents.write(to: target, options: .withoutOverwriting)
            } catch let error as NSError where error.domain == NSCocoaErrorDomain
                && error.code == NSFileWriteFileExistsError {
                failure = CoordinatorError.AlreadyExists(message: path)
            } catch {
                failure = error
            }
        }
        try Self.check(coordinationError ?? failure, path: path)
        Log.info("coordinator: create \(path) in \(millis(since: start))ms")
    }

    func coordinatedCreateDirectory(path: String) throws {
        var coordinationError: NSError?
        var failure: Error?
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path, directoryHint: .isDirectory), options: [], error: &coordinationError
        ) { target in
            do {
                try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
            } catch { failure = error }
        }
        try Self.check(coordinationError ?? failure, path: path)
        Log.info("coordinator: mkdir \(path)")
    }

    func coordinatedRemove(path: String) throws {
        var coordinationError: NSError?
        var failure: Error?
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path), options: .forDeleting, error: &coordinationError
        ) { target in
            do { try FileManager.default.removeItem(at: target) } catch { failure = error }
        }
        try Self.check(coordinationError ?? failure, path: path)
        Log.info("coordinator: remove \(path)")
    }

    /// Returns at once; the downloads start on a background queue. Ubiquitous
    /// items (iCloud Drive, and replicated File Providers: Dropbox, OneDrive)
    /// are asked to download; anything else gets a coordinated read, which is
    /// what makes an older third-party provider materialize the file.
    func requestDownload(paths: [String]) {
        Log.info("coordinator: download requested for \(paths.count) file(s)")
        DispatchQueue.global(qos: .utility).async {
            for path in paths {
                Self.download(URL(filePath: path))
            }
        }
    }

    // MARK: Helpers

    /// Throws `Changed` unless `target` is a regular file (not a symlink,
    /// not missing) holding `expectedLen` bytes that hash to `expectedHash`.
    static func checkUnchanged(_ target: URL, expectedLen: UInt64, expectedHash: String) throws {
        let path = target.path(percentEncoded: false)
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: path),
              attributes[.type] as? FileAttributeType == .typeRegular
        else {
            throw CoordinatorError.Changed(message: "\(path) is missing or not a regular file")
        }
        let current: Data
        do {
            current = try Data(contentsOf: target)
        } catch {
            throw CoordinatorError.Changed(message: "\(path) could not be read back: \(error.localizedDescription)")
        }
        guard UInt64(current.count) == expectedLen, FNV1a.hex(current) == expectedHash.lowercased() else {
            throw CoordinatorError.Changed(message: "\(path) changed since it was read")
        }
    }

    /// Throws the failure of a coordinated operation, if there was one, as a
    /// `CoordinatorError`.
    private static func check(_ error: Error?, path: String) throws {
        guard let error else { return }
        Log.info("coordinator: \(path) failed: \(error.localizedDescription)")
        throw self.coordinatorError(error, path: path)
    }

    /// `replaceItemAt` from a sibling in the item-replacement directory for
    /// the target's volume (never a move out of `NSTemporaryDirectory()`,
    /// which may be another volume).
    private static func replace(_ target: URL, with contents: Data) throws {
        let fileManager = FileManager.default
        let scratch = try fileManager.url(
            for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: target, create: true
        )
        defer { try? fileManager.removeItem(at: scratch) }
        let staged = scratch.appending(path: target.lastPathComponent)
        try contents.write(to: staged)
        _ = try fileManager.replaceItemAt(target, withItemAt: staged)
    }

    private static func download(_ url: URL) {
        let path = url.path(percentEncoded: false)
        let ubiquitous = (try? url.resourceValues(forKeys: [.isUbiquitousItemKey]))?.isUbiquitousItem ?? false
        if ubiquitous {
            do {
                try FileManager.default.startDownloadingUbiquitousItem(at: url)
                Log.info("coordinator: download started for \(path)")
            } catch {
                Log.info("coordinator: download of \(path) failed: \(error.localizedDescription)")
            }
            return
        }
        var coordinationError: NSError?
        NSFileCoordinator(filePresenter: nil).coordinate(readingItemAt: url, options: [], error: &coordinationError) {
            _ = try? Data(contentsOf: $0, options: .mappedIfSafe)
        }
        if let coordinationError {
            Log.info("coordinator: coordinated read of \(path) failed: \(coordinationError.localizedDescription)")
        }
    }

    static func coordinatorError(_ error: Error, path: String) -> CoordinatorError {
        if let error = error as? CoordinatorError { return error }
        let nsError = error as NSError
        let message = "\(path): \(nsError.localizedDescription)"
        let underlying = nsError.userInfo[NSUnderlyingErrorKey] as? NSError
        for candidate in [nsError, underlying].compactMap(\.self) {
            switch (candidate.domain, candidate.code) {
            case (NSCocoaErrorDomain, NSFileNoSuchFileError),
                 (NSCocoaErrorDomain, NSFileReadNoSuchFileError),
                 (NSPOSIXErrorDomain, Int(ENOENT)):
                return .NotFound(message: message)
            case (NSCocoaErrorDomain, NSFileWriteFileExistsError),
                 (NSPOSIXErrorDomain, Int(EEXIST)):
                return .AlreadyExists(message: message)
            case (NSCocoaErrorDomain, NSFileWriteNoPermissionError),
                 (NSCocoaErrorDomain, NSFileReadNoPermissionError),
                 (NSPOSIXErrorDomain, Int(EACCES)),
                 (NSPOSIXErrorDomain, Int(EPERM)):
                return .PermissionDenied(message: message)
            default:
                continue
            }
        }
        return .Failed(message: message)
    }
}
