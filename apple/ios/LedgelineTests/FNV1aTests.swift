import Foundation
import XCTest

/// The FNV-1a 64 test vectors from crates/ledgeline-ffi/README.md: the
/// conditional replace compares this hash with the one Rust computed, so a
/// mismatch would refuse every save as "changed".
final class FNV1aTests: XCTestCase {
    func testReadmeVectors() {
        XCTAssertEqual(FNV1a.hex(Data()), "cbf29ce484222325")
        XCTAssertEqual(FNV1a.hex(Data("a".utf8)), "af63dc4c8601ec8c")
        XCTAssertEqual(FNV1a.hex(Data("foobar".utf8)), "85944171f73967e8")
        XCTAssertTrue(FNV1a.selfCheck())
    }

    func testHexIsSixteenLowercaseDigits() {
        for sample in ["", "x", "hledger", String(repeating: "z", count: 4096)] {
            let hex = FNV1a.hex(Data(sample.utf8))
            XCTAssertEqual(hex.count, 16)
            XCTAssertEqual(hex, hex.lowercased())
        }
    }
}
