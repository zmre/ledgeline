import Foundation

/// FNV-1a 64-bit, as the Rust core fingerprints file contents
/// (`ledgeline_core::vault::Fingerprint`). `FileCoordinatorBridge` compares a
/// file's hash with the one Rust expects before replacing it; the encoding is
/// 16 lowercase hex digits, zero-padded (crates/ledgeline-ffi/README.md).
enum FNV1a {
    static let offsetBasis: UInt64 = 0xCBF2_9CE4_8422_2325
    static let prime: UInt64 = 0x0000_0100_0000_01B3

    static func hash(_ data: Data) -> UInt64 {
        var hash = self.offsetBasis
        for byte in data {
            hash ^= UInt64(byte)
            hash = hash &* self.prime
        }
        return hash
    }

    /// `hash(data)` as 16 lowercase hex digits.
    static func hex(_ data: Data) -> String {
        let digits = String(self.hash(data), radix: 16)
        return String(repeating: "0", count: max(0, 16 - digits.count)) + digits
    }

    /// The README's test vectors. `LedgelineTests` checks them; DEBUG builds
    /// also check them once at launch, so a broken hash shows up before the
    /// first save is refused as "changed".
    static let vectors: [(String, String)] = [
        ("", "cbf29ce484222325"),
        ("a", "af63dc4c8601ec8c"),
        ("foobar", "85944171f73967e8"),
    ]

    static func selfCheck() -> Bool {
        self.vectors.allSatisfy { self.hex(Data($0.0.utf8)) == $0.1 }
    }
}
