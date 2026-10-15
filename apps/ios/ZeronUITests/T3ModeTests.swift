import XCTest

/// T3 Mode over the core's demo workspace: an Inbox, folded shelves, and
/// Snooze and Settle reachable by touch alone (swipe, ⋯ and long-press).
/// There is no Wake control: a snooze ends at its deadline or when the user
/// sends the session a message (just opening it is not enough).
final class T3ModeTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        // `-t3` seeds the in-memory mode the demo uses; nothing persists.
        app.launchArguments = ["-demo", "-fast", "-t3"]
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    /// The first session row under the Inbox header.
    private func inboxRow(_ app: XCUIApplication) -> XCUIElement {
        app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'session-'")).firstMatch
    }

    func testSnoozeSendAndSettleByTouch() {
        let app = launch()
        let inbox = app.cells["section-recent"]
        XCTAssertTrue(inbox.waitForExistence(timeout: 10))
        XCTAssertTrue(inbox.label.hasPrefix("Inbox"))
        snapshot(app, "t3-front")

        // Swipe offers Settle and Snooze; Snooze opens the presets.
        let row = inboxRow(app)
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        let id = row.identifier
        row.swipeLeft()
        XCTAssertTrue(app.buttons["Settle"].waitForExistence(timeout: 3))
        app.buttons["Snooze"].tap()
        let hour = app.buttons["1 Hour"]
        XCTAssertTrue(hour.waitForExistence(timeout: 3))
        snapshot(app, "t3-snooze-sheet")
        hour.tap()

        // It waits on the folded Snoozed shelf, showing when it comes back.
        let snoozed = app.cells["section-t3-snoozed"]
        XCTAssertTrue(snoozed.waitForExistence(timeout: 5))
        XCTAssertEqual(snoozed.value as? String, "Collapsed")
        XCTAssertFalse(app.cells[id].exists)
        snoozed.tap()
        XCTAssertEqual(snoozed.value as? String, "Expanded")
        let parked = app.cells[id]
        XCTAssertTrue(parked.waitForExistence(timeout: 3))
        XCTAssertTrue(parked.label.contains("Snoozed, wakes in"))
        snapshot(app, "t3-snoozed-shelf")

        // No Wake control anywhere on the row.
        parked.press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["Snooze"].waitForExistence(timeout: 3), "re-timing stays available")
        XCTAssertFalse(app.buttons["Wake Now"].exists)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.06)).tap()

        // Just reading it keeps the snooze: back on the list it is still parked.
        parked.tap()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(snoozed.waitForExistence(timeout: 5))
        XCTAssertTrue(app.cells[id].waitForExistence(timeout: 3))
        XCTAssertTrue(app.cells[id].label.contains("Snoozed, wakes in"))

        // Sending it a message ends the snooze: it is back in the inbox.
        app.cells[id].tap()
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap()
        input.typeText("Picking this back up.")
        app.buttons["composer-send"].tap()
        let echo = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH 'You: Picking this back up.'")).firstMatch
        XCTAssertTrue(echo.waitForExistence(timeout: 5))
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(snoozed.waitForNonExistence(timeout: 10))
        let restored = app.cells[id]
        XCTAssertTrue(restored.waitForExistence(timeout: 3))
        XCTAssertFalse(restored.label.contains("Snoozed"))

        // A full Settle lands on the folded Settled shelf, undoable from there.
        app.cells[id].swipeLeft()
        app.buttons["Settle"].tap()
        let settled = app.cells["section-t3-settled"]
        XCTAssertTrue(settled.waitForExistence(timeout: 5))
        XCTAssertEqual(settled.value as? String, "Collapsed")
        settled.tap()
        let done = app.cells[id]
        XCTAssertTrue(done.waitForExistence(timeout: 3))
        done.swipeLeft()
        app.buttons["Unsettle"].tap()
        XCTAssertTrue(app.cells[id].waitForExistence(timeout: 5))
        snapshot(app, "t3-unsettled")
    }

    func testRowActionsButtonOffersSnoozeAndSettle() {
        let app = launch()
        let row = inboxRow(app)
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        let id = row.identifier
        let actions = app.buttons["session-actions-" + id.dropFirst("session-".count)]
        XCTAssertTrue(actions.waitForExistence(timeout: 3))
        XCTAssertTrue(actions.label.hasPrefix("Actions for "))
        actions.tap()
        // The menu opens in place: the row is not opened.
        XCTAssertTrue(app.buttons["Settle"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.buttons["Snooze"].exists)
        XCTAssertFalse(app.textViews["composer-input"].exists)
        snapshot(app, "t3-row-actions")
        app.buttons["Snooze"].tap()
        XCTAssertTrue(app.buttons["1 Hour"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["1 Week"].exists)

        // Past the presets: wake times, not a duration form.
        // Menu items with a subtitle may fold it into their label.
        XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Tomorrow Morning'")).firstMatch.exists)
        XCTAssertFalse(app.buttons["Custom…"].exists)
        snapshot(app, "t3-snooze-until-menu")
        app.buttons["Pick Date & Time…"].tap()

        // The sheet opens on tomorrow morning and names the exact wake time.
        let confirm = app.buttons["snooze-until-confirm"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        XCTAssertTrue(confirm.isEnabled)
        XCTAssertTrue(confirm.label.hasPrefix("Snooze until "))
        XCTAssertTrue(app.datePickers["snooze-until-picker"].exists)
        snapshot(app, "t3-snooze-until-picker")
        confirm.tap()

        // The row parks on the folded Snoozed shelf until then.
        let snoozed = app.cells["section-t3-snoozed"]
        XCTAssertTrue(snoozed.waitForExistence(timeout: 5))
        XCTAssertEqual(snoozed.value as? String, "Collapsed")
        XCTAssertFalse(app.cells[id].exists)
        snoozed.tap()
        XCTAssertTrue(app.cells[id].waitForExistence(timeout: 3))
        XCTAssertTrue(app.cells[id].label.contains("Snoozed, wakes in"))
    }

    func testOptionsMenuTogglesBackToDefault() {
        let app = launch()
        let inbox = app.cells.matching(NSPredicate(format: "label BEGINSWITH 'Inbox'")).firstMatch
        XCTAssertTrue(inbox.waitForExistence(timeout: 10))
        app.buttons["sessions-options"].tap()
        let t3 = app.buttons["T3 Mode"]
        XCTAssertTrue(t3.waitForExistence(timeout: 3))
        t3.tap()
        // Default: no Inbox header, no shelves.
        XCTAssertTrue(inbox.waitForNonExistence(timeout: 5))
        // Default rows keep their plain layout: no ⋯ button.
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'session-actions-'")).firstMatch.exists)
        for shelf in ["section-t3-working", "section-t3-snoozed", "section-t3-settled"] {
            XCTAssertFalse(app.cells[shelf].exists)
        }
    }

    // MARK: Screenshots

    /// Native captures for review (CI: `ios_t3_screenshots`): the folded
    /// Inbox, a row's ⋯ Snooze menu, then every shelf open, in dark and light.
    /// Defensive by design — a missing optional element skips its shot, never
    /// the rest of the tour.
    func testCaptureT3Screens() {
        for (look, style) in [("dark", "2"), ("light", "1")] {
            let app = XCUIApplication()
            // `-appearance` lands in the argument domain the scene reads.
            app.launchArguments = ["-demo", "-fast", "-t3", "-appearance", style]
            app.launch()
            let inbox = app.cells.matching(NSPredicate(format: "label BEGINSWITH 'Inbox'")).firstMatch
            XCTAssertTrue(inbox.waitForExistence(timeout: 20), "T3 Inbox header")
            settle(app)
            capture(app, "t3-\(look)-inbox-folded")

            // The row's ⋯ menu, then its Snooze presets.
            if openRowActions(app) {
                capture(app, "t3-\(look)-row-actions")
                let snooze = app.buttons["Snooze"]
                if snooze.waitForExistence(timeout: 3) {
                    snooze.tap()
                    if app.buttons["1 Hour"].waitForExistence(timeout: 3) {
                        capture(app, "t3-\(look)-row-snooze-menu")
                        // The date & time sheet; confirming parks the row so
                        // the Snoozed shelf has something to show.
                        app.buttons["Pick Date & Time…"].tap()
                        let confirm = app.buttons["snooze-until-confirm"]
                        if confirm.waitForExistence(timeout: 5) {
                            settle(app)
                            capture(app, "t3-\(look)-snooze-until-picker")
                            confirm.tap()
                        }
                    }
                }
                dismissMenus(app)
            }
            // Settle another so the Settled shelf is populated too.
            if openRowActions(app), app.buttons["Settle"].waitForExistence(timeout: 3) {
                app.buttons["Settle"].tap()
            }
            dismissMenus(app)
            settle(app)
            capture(app, "t3-\(look)-shelves-folded")

            // Open every shelf, then follow the list down.
            for shelf in ["section-t3-working", "section-t3-snoozed", "section-t3-settled"] {
                let header = app.cells[shelf]
                if !header.exists { app.swipeUp() }
                if header.waitForExistence(timeout: 2), header.value as? String == "Collapsed" {
                    header.tap()
                }
            }
            settle(app)
            capture(app, "t3-\(look)-shelves-open")
            app.swipeUp()
            settle(app)
            capture(app, "t3-\(look)-shelves-open-scrolled")
            app.terminate()
        }
    }

    /// Tap the first row's ⋯; fall back to its right edge if the accessory
    /// is folded into the cell's own accessibility element.
    private func openRowActions(_ app: XCUIApplication) -> Bool {
        let button = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'session-actions-'")).firstMatch
        if button.waitForExistence(timeout: 3) {
            button.tap()
        } else {
            let row = inboxRow(app)
            guard row.waitForExistence(timeout: 3) else { return false }
            row.coordinate(withNormalizedOffset: CGVector(dx: 0.96, dy: 0.5)).tap()
        }
        return app.buttons["Settle"].waitForExistence(timeout: 3) || app.buttons["Snooze"].exists
    }

    /// Close any open menu by tapping the navigation bar's empty middle.
    private func dismissMenus(_ app: XCUIApplication) {
        guard app.collectionViews.firstMatch.exists else { return }
        if app.buttons["Snooze"].exists || app.buttons["Settle"].exists || app.buttons["1 Hour"].exists {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.06)).tap()
        }
    }

    /// Let toasts, list diffs and menu fades come to rest before a capture.
    private func settle(_ app: XCUIApplication) {
        _ = app.wait(for: .runningForeground, timeout: 1)
        Thread.sleep(forTimeInterval: 1.2)
    }

    private func capture(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
