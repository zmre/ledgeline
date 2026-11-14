import CoreGraphics

/// Turns a pinch's running scale into UI-scale steps (plan 42): one step each
/// time the fingers spread (or close) by `ratio` since the last step. Pure, so
/// the thresholds are unit-tested without a finger.
struct PinchSteps {
    /// The `ledgeline:zoom` event's `detail` (web/src/lib/theme/zoom.svelte.ts).
    enum Direction: String {
        case zoomIn = "in"
        case zoomOut = "out"
    }

    /// How far a pinch travels per step: a quarter wider (or narrower).
    static let ratio: CGFloat = 1.25

    /// The pinch scale at the last step (1 at the start of a pinch).
    private(set) var anchor: CGFloat = 1

    /// The steps a pinch now at `scale` has earned since the last call.
    mutating func advance(to scale: CGFloat) -> [Direction] {
        guard scale.isFinite, scale > 0 else { return [] }
        var steps: [Direction] = []
        while scale >= self.anchor * Self.ratio {
            self.anchor *= Self.ratio
            steps.append(.zoomIn)
        }
        while scale <= self.anchor / Self.ratio {
            self.anchor /= Self.ratio
            steps.append(.zoomOut)
        }
        return steps
    }
}
