import XCTest
@testable import Zeron

/// The effort slider's pure parts: tiers, heat, track geometry, motion and
/// the spark field (matching Android's `EffortSlider.kt`).
final class EffortSliderTests: XCTestCase {
    func testTopTiers() {
        for level in ["ultra", "ultracode", "ultrathink"] { XCTAssertTrue(EffortScale.isPeak(level), level) }
        for level in ["low", "medium", "high", "xhigh", "max", ""] { XCTAssertFalse(EffortScale.isPeak(level), level) }
    }

    func testHeatHitsItsStopsAndInterpolates() {
        XCTAssertEqual(EffortScale.heat(0), EffortScale.RGB(hex: 0x38BDF8))
        XCTAssertEqual(EffortScale.heat(0.35), EffortScale.RGB(hex: 0x3B82F6))
        XCTAssertEqual(EffortScale.heat(1), EffortScale.RGB(hex: 0xD946EF))
        // Clamped outside 0…1.
        XCTAssertEqual(EffortScale.heat(-1), EffortScale.heat(0))
        XCTAssertEqual(EffortScale.heat(2), EffortScale.heat(1))
        // Halfway between the 0.6 and 0.82 stops.
        let mid = EffortScale.heat(0.71)
        let a = EffortScale.RGB(hex: 0x6366F1), b = EffortScale.RGB(hex: 0x8B5CF6)
        XCTAssertEqual(mid.r, (a.r + b.r) / 2, accuracy: 0.001)
        XCTAssertEqual(mid.b, (a.b + b.b) / 2, accuracy: 0.001)
    }

    func testAccentNeverLighterThanBlue() {
        XCTAssertEqual(EffortScale.accent(fraction: 0), EffortScale.RGB(hex: 0x3B82F6))
        XCTAssertEqual(EffortScale.accent(fraction: 1), EffortScale.RGB(hex: 0xD946EF))
    }

    func testFillBandEndsAtTheLevel() {
        let low = EffortScale.fillBand(fraction: 0.2)
        XCTAssertEqual(low.count, 7)
        XCTAssertEqual(low.first, EffortScale.heat(0))
        XCTAssertEqual(low.last, EffortScale.heat(0.2))
        let top = EffortScale.fillBand(fraction: 1)
        XCTAssertEqual(top.first, EffortScale.heat(0.5))
        XCTAssertEqual(top.last, EffortScale.heat(1))
    }

    func testRampAndGlow() {
        XCTAssertEqual(EffortScale.ramp(0.55, 1, 0.5), 0)
        XCTAssertEqual(EffortScale.ramp(0.55, 1, 1), 1)
        XCTAssertEqual(EffortScale.ramp(0, 1, 0.5), 0.5, accuracy: 0.0001)
        XCTAssertEqual(EffortScale.glow(fraction: 0.4, breath: 0.25, top: 1), 0)
        // Full effort, breath at its peak, a top tier: full strength.
        XCTAssertEqual(EffortScale.glow(fraction: 1, breath: 0.25, top: 1), 1, accuracy: 0.0001)
        XCTAssertLessThan(EffortScale.glow(fraction: 1, breath: 0.25, top: 0), 1)
    }

    func testStepFromPosition() {
        // 4 levels on a 244×44 track: the thumb runs from x = 22 to x = 222.
        let at = { (x: CGFloat) in EffortScale.step(atX: x, width: 244, height: 44, count: 4) }
        XCTAssertEqual(at(0), 0)
        XCTAssertEqual(at(22), 0)
        XCTAssertEqual(at(22 + 200 / 3), 1)
        XCTAssertEqual(at(22 + 200 / 2 + 1), 2)
        XCTAssertEqual(at(222), 3)
        XCTAssertEqual(at(1000), 3)
        XCTAssertEqual(EffortScale.step(atX: 100, width: 244, height: 44, count: 1), 0)
        XCTAssertEqual(EffortScale.fraction(ofStep: 2, count: 5), 0.5)
        XCTAssertEqual(EffortScale.fraction(ofStep: 0, count: 1), 0)
    }

    func testSparksThickenWithEffort() {
        XCTAssertEqual(EffortScale.visibleSparks(fraction: 0), 8)
        XCTAssertEqual(EffortScale.visibleSparks(fraction: 0.5), 26)
        XCTAssertEqual(EffortScale.visibleSparks(fraction: 1), 44)
        XCTAssertEqual(EffortScale.visibleSparks(fraction: 3), 44)
        XCTAssertEqual(EffortScale.streak(fraction: 0.5), 0)
        XCTAssertEqual(EffortScale.streak(fraction: 1), 14)
    }

    func testSparkFieldIsSeededAndInRange() {
        var a = EffortScale.SeededRandom(seed: 7), b = EffortScale.SeededRandom(seed: 7)
        XCTAssertEqual((0..<8).map { _ in a.next() }, (0..<8).map { _ in b.next() })
        XCTAssertEqual(EffortScale.sparks.count, 44)
        for s in EffortScale.sparks {
            XCTAssert((0..<1).contains(s.x))
            XCTAssert((0.18...0.82).contains(s.y))
            XCTAssert((0.8...2.2).contains(s.radius))
            XCTAssert((0.5...1.5).contains(s.speed))
            XCTAssert((1...3).contains(s.twinkle))
            let x = EffortScale.sparkX(s, drift: 12.3)
            XCTAssert((0..<1).contains(x))
        }
    }

    /// The session's effort chip shows whenever the model has a ladder,
    /// naming the session's level, else the model's default, else the middle.
    func testSessionEffortChipLevel() {
        let ladder = ["low", "medium", "high", "max"]
        XCTAssertEqual(CoreSessionSource.effortLevel(reasoning: "low", levels: ladder, defaultLevel: "high"), "low")
        XCTAssertEqual(CoreSessionSource.effortLevel(reasoning: nil, levels: ladder, defaultLevel: "high"), "high")
        XCTAssertEqual(CoreSessionSource.effortLevel(reasoning: "", levels: ladder, defaultLevel: nil), "high")
        XCTAssertEqual(CoreSessionSource.effortLevel(reasoning: nil, levels: ladder, defaultLevel: "ultra"), "high", "a default off the ladder")
        XCTAssertNil(CoreSessionSource.effortLevel(reasoning: nil, levels: [], defaultLevel: "high"))
        XCTAssertEqual(CoreSessionSource.effortLevel(reasoning: "xhigh", levels: [], defaultLevel: nil), "xhigh")
    }

    func testMotionQuickensWithIntensityAndWraps() {
        var calm = EffortScale.Motion(), hot = EffortScale.Motion()
        calm.advance(1, intensity: 0)
        hot.advance(1, intensity: 1)
        XCTAssertEqual(calm.drift, 0.03, accuracy: 1e-9)
        XCTAssertEqual(hot.drift, 0.2, accuracy: 1e-9)
        XCTAssertEqual(calm.breath, 0.4, accuracy: 1e-9)
        XCTAssertEqual(hot.breath, 1, accuracy: 1e-9)
        // 1.2 s per sweep at full intensity: 2 s wraps to 0.8 / 1.2.
        hot.advance(1, intensity: 1)
        XCTAssertEqual(hot.sweep, 2 / 1.2 - 1, accuracy: 1e-9)
        XCTAssert((0..<1).contains(hot.sweep))
    }
}
