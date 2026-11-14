import CoreGraphics
import XCTest

/// A pinch steps the SPA's UI scale once per `PinchSteps.ratio` of travel.
final class PinchStepsTests: XCTestCase {
    func testSmallPinchesDoNothing() {
        var steps = PinchSteps()
        XCTAssertEqual(steps.advance(to: 1.1), [])
        XCTAssertEqual(steps.advance(to: 0.9), [])
    }

    func testOneStepPerRatioEitherWay() {
        var steps = PinchSteps()
        XCTAssertEqual(steps.advance(to: 1.25), [.zoomIn])
        XCTAssertEqual(steps.advance(to: 1.3), [])
        XCTAssertEqual(steps.advance(to: 1.25 * 1.25), [.zoomIn])
        // Back down to where it started: two steps out.
        XCTAssertEqual(steps.advance(to: 1.0), [.zoomOut, .zoomOut])
    }

    func testAFastPinchEarnsEveryStepAtOnce() {
        var steps = PinchSteps()
        XCTAssertEqual(steps.advance(to: 2.0), [.zoomIn, .zoomIn, .zoomIn])
        var closing = PinchSteps()
        XCTAssertEqual(closing.advance(to: 0.5), [.zoomOut, .zoomOut, .zoomOut])
    }

    func testNonsenseScalesAreIgnored() {
        var steps = PinchSteps()
        XCTAssertEqual(steps.advance(to: 0), [])
        XCTAssertEqual(steps.advance(to: .nan), [])
        XCTAssertEqual(steps.advance(to: .infinity), [])
    }

    func testDirectionsAreTheZoomEventsDetail() {
        XCTAssertEqual(PinchSteps.Direction.zoomIn.rawValue, "in")
        XCTAssertEqual(PinchSteps.Direction.zoomOut.rawValue, "out")
    }
}
