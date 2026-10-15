import XCTest
@testable import Zeron

/// The custom snooze dialog's validation: a positive whole number whose span
/// fits the core's `UInt32` seconds.
final class CustomSnoozeTests: XCTestCase {
    private let minute: UInt32 = 60
    private let day: UInt32 = 24 * 60 * 60

    func testPositiveWholeNumbersScaleByUnit() {
        XCTAssertEqual(CustomSnooze.seconds("90", unit: minute), 5_400)
        XCTAssertEqual(CustomSnooze.seconds(" 2 ", unit: day), 172_800)
    }

    func testZeroAndNonWholeNumbersAreRejected() {
        for text in ["", "0", "-5", "1.5", "1e3", "ten", "3 hours"] {
            XCTAssertNil(CustomSnooze.seconds(text, unit: minute), text)
        }
    }

    func testSpansPastUInt32SecondsAreRejected() {
        // 49710 days is the last whole day under UInt32.max seconds.
        XCTAssertEqual(CustomSnooze.seconds("49710", unit: day), 49_710 * day)
        XCTAssertNil(CustomSnooze.seconds("49711", unit: day))
        XCTAssertNil(CustomSnooze.seconds("4294967296", unit: 1))
        XCTAssertEqual(CustomSnooze.seconds("4294967295", unit: 1), .max)
    }

    func testTitlesAgreeInNumber() {
        XCTAssertEqual(CustomSnooze.title("1", unit: "Hours"), "1 Hour")
        XCTAssertEqual(CustomSnooze.title("90", unit: "Minutes"), "90 Minutes")
    }
}
