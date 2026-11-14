import Foundation
import os

/// The app's log: the unified log (Console.app, subsystem
/// `com.zmre.ledgeline.ios`) plus stderr, which shows up at once under
/// `simctl launch --console-pty`.
///
/// Never log journal contents or amounts: paths, timings and statuses only.
enum Log {
    private static let logger = Logger(subsystem: "com.zmre.ledgeline.ios", category: "app")

    static func info(_ message: String) {
        self.logger.notice("\(message, privacy: .public)")
        // The throwing `write(contentsOf:)`, never `write(_:)`: that one raises
        // an Objective-C exception on any error, and stderr becomes a closed
        // pipe (EPIPE) when an attached console (`devicectl … --console`) goes
        // away; the phone sleeping was enough to abort mbr's app this way.
        try? FileHandle.standardError.write(contentsOf: Data("[ledgeline] \(message)\n".utf8))
    }
}

/// Milliseconds elapsed since `start`.
func millis(since start: ContinuousClock.Instant) -> Int {
    let elapsed = ContinuousClock.now - start
    return Int(elapsed.components.seconds * 1000)
        + Int(elapsed.components.attoseconds / 1_000_000_000_000_000)
}
