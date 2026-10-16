import XCTest
@testable import Zeron

/// The model picker's "More models" fold: older generations hide by default,
/// but never the selected one, and an all-or-nothing list stays flat.
final class ModelPickerFoldTests: XCTestCase {
    /// Ids ending in `-old` are legacy.
    private func fold(_ ids: [String], selected: String? = nil) -> (shown: [String], more: [String]) {
        foldLegacy(ids, legacy: { $0.hasSuffix("-old") }, selected: { $0 == selected })
    }

    func testLegacyModelsFoldByDefault() {
        let f = fold(["a", "b-old", "c", "d-old"])
        XCTAssertEqual(f.shown, ["a", "c"])
        XCTAssertEqual(f.more, ["b-old", "d-old"])
    }

    func testSelectedLegacyModelStaysInline() {
        let f = fold(["a", "b-old", "c", "d-old"], selected: "d-old")
        XCTAssertEqual(f.shown, ["a", "c", "d-old"])
        XCTAssertEqual(f.more, ["b-old"])
    }

    func testNoFoldWhenAllOrNoneAreLegacy() {
        XCTAssertEqual(fold(["a", "b"]).shown, ["a", "b"])
        XCTAssertEqual(fold(["a", "b"]).more, [])
        XCTAssertEqual(fold(["a-old", "b-old"]).shown, ["a-old", "b-old"])
        XCTAssertEqual(fold(["a-old", "b-old"]).more, [])
    }

    func testVendorGroupsFoldIndependently() {
        // Each vendor submenu folds on its own: Anthropic's lone legacy model
        // stays inline, OpenAI's sits behind "More models".
        let groups = vendorGroups(["openai/gpt-5.6", "openai/gpt-5.5-old", "anthropic/claude-opus-4.7-old"], harness: "opencode", id: { $0 }) ?? []
        XCTAssertEqual(groups.map(\.vendor), [.anthropic, .openai])
        let folds = groups.map { fold($0.models) }
        guard folds.count == 2 else { return XCTFail("expected two vendor groups") }
        XCTAssertEqual(folds[0].more, [])
        XCTAssertEqual(folds[1].shown, ["openai/gpt-5.6"])
        XCTAssertEqual(folds[1].more, ["openai/gpt-5.5-old"])
    }
}
