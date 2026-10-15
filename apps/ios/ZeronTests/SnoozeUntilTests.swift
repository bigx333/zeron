import XCTest
@testable import Zeron

/// "Snooze until": the contextual wake times and their conversion to the
/// core's whole-minute `UInt32` seconds. Pinned to UTC and a fixed locale.
final class SnoozeUntilTests: XCTestCase {
    private var calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }()

    /// 2026-10-14 (a Wednesday) at `hour:minute` UTC, plus `days`.
    private func at(_ hour: Int, _ minute: Int = 0, days: Int = 0) -> Date {
        calendar.date(from: DateComponents(year: 2026, month: 10, day: 14 + days, hour: hour, minute: minute))!
    }

    func testMiddayOffersEveningTomorrowAndNextWeek() {
        let s = SnoozeUntil.suggestions(now: at(10), calendar: calendar)
        XCTAssertEqual(s.map(\.title), ["This Evening", "Tomorrow Morning", "Next Week"])
        XCTAssertEqual(s.map(\.date), [at(18), at(9, days: 1), at(9, days: 5)])
    }

    func testEveningDropsOnceUnderAnHourAway() {
        XCTAssertEqual(SnoozeUntil.suggestions(now: at(17, 30), calendar: calendar).map(\.title), ["Tomorrow Morning", "Next Week"])
        XCTAssertEqual(SnoozeUntil.suggestions(now: at(22), calendar: calendar).map(\.title), ["Tomorrow Morning", "Next Week"])
    }

    func testNextWeekIsOmittedWhenItIsTomorrow() {
        // Sunday 2026-10-18: tomorrow morning is already Monday.
        XCTAssertEqual(SnoozeUntil.suggestions(now: at(10, days: 4), calendar: calendar).map(\.title), ["This Evening", "Tomorrow Morning"])
        // Monday: next week is the following Monday, not today.
        let monday = SnoozeUntil.suggestions(now: at(8, days: 5), calendar: calendar)
        XCTAssertEqual(monday.last?.date, at(9, days: 12))
    }

    func testSecondsRoundUpToWholeMinutesWithinUInt32() {
        let now = at(10)
        XCTAssertEqual(SnoozeUntil.seconds(until: now.addingTimeInterval(30), now: now), 60)
        XCTAssertEqual(SnoozeUntil.seconds(until: now.addingTimeInterval(61), now: now), 120)
        XCTAssertEqual(SnoozeUntil.seconds(until: at(18), now: now), 8 * 3600)
        XCTAssertNil(SnoozeUntil.seconds(until: now, now: now))
        XCTAssertNil(SnoozeUntil.seconds(until: now.addingTimeInterval(-60), now: now))
        XCTAssertNil(SnoozeUntil.seconds(until: now.addingTimeInterval(TimeInterval(UInt32.max) + 60), now: now))
    }

    /// Native punctuation varies by OS release ("Oct 24, 9:00 AM" or
    /// "Oct 24 at 9:00 AM", narrow no-break spaces): check the parts.
    func testLabelsNameTheDayOnlyWhenNeeded() {
        func label(_ date: Date, _ locale: String = "en_US") -> String {
            SnoozeUntil.label(date, now: at(10), calendar: calendar, locale: Locale(identifier: locale))
                .replacingOccurrences(of: "\u{202F}", with: " ")
        }
        func assertParts(_ text: String, has: [String], lacks: [String], line: UInt = #line) {
            for part in has { XCTAssertTrue(text.contains(part), "\(text) lacks \(part)", line: line) }
            for part in lacks { XCTAssertFalse(text.contains(part), "\(text) has \(part)", line: line) }
        }
        // Today: the time alone.
        assertParts(label(at(18)), has: ["6:00", "PM"], lacks: ["Wed", "Oct", "2026"])
        // This week: weekday and time.
        assertParts(label(at(9, days: 1)), has: ["Thu", "9:00", "AM"], lacks: ["Oct", "2026"])
        // Beyond the week: month and day, no weekday, no year.
        assertParts(label(at(9, days: 10)), has: ["Oct", "24", "9:00", "AM"], lacks: ["Sat", "2026"])
        // Another year: the year appears.
        assertParts(label(at(10, days: 90)), has: ["Jan", "12", "2027", "10:00"], lacks: [])
        // A 24-hour locale drops AM/PM.
        assertParts(label(at(18), "en_GB"), has: ["18:00"], lacks: ["PM", "pm"])
    }
}
