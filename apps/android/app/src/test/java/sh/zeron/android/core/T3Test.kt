package sh.zeron.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.time.ZoneId
import java.time.ZonedDateTime
import java.util.Locale

class T3Test {
    @Test
    fun modeParsesStoredNamesAndFallsBackToDefault() {
        assertEquals(SidebarMode.T3, SidebarMode.parse("T3"))
        assertEquals(SidebarMode.Default, SidebarMode.parse("Default"))
        assertEquals(SidebarMode.Default, SidebarMode.parse(null))
        assertEquals(SidebarMode.Default, SidebarMode.parse("t3-mode"))
    }

    @Test
    fun settleWordingFollowsTheMode() {
        assertEquals("Settle", SidebarMode.T3.settle)
        assertEquals("Unsettle", SidebarMode.T3.unsettle)
        assertEquals("Settled", SidebarMode.T3.settled)
        assertEquals("Archive", SidebarMode.Default.settle)
        assertEquals("Unarchive", SidebarMode.Default.unsettle)
        assertEquals("Archived", SidebarMode.Default.settled)
    }

    @Test
    fun snoozePresetsAreTheDesktopLengths() {
        assertEquals(listOf("15m", "1h", "4h", "1d"), SnoozeOptions.map { it.short })
        assertEquals(listOf(900u, 3_600u, 14_400u, 86_400u), SnoozeOptions.map { it.seconds })
        assertEquals("for 1 hour", SnoozeOptions[1].phrase)
    }

    private val zone = ZoneId.of("America/New_York")
    private fun at(day: Int, hour: Int, minute: Int = 0) = ZonedDateTime.of(2026, 10, day, hour, minute, 0, 0, zone)

    @Test
    fun shortcutsAreEveningTomorrowAndNextWeek() {
        // Wednesday 10:00.
        val now = at(14, 10)
        assertEquals(
            listOf("This evening" to at(14, 18), "Tomorrow morning" to at(15, 9), "Next week" to at(19, 9)),
            snoozeShortcuts(now).map { it.title to it.at },
        )
    }

    @Test
    fun eveningNeedsMoreThanAnHourAndNextWeekIsntTomorrow() {
        assertEquals(true, snoozeShortcuts(at(14, 16, 59)).any { it.title == "This evening" })
        assertEquals(false, snoozeShortcuts(at(14, 17)).any { it.title == "This evening" })
        assertEquals(false, snoozeShortcuts(at(14, 22)).any { it.title == "This evening" })
        // Sunday: Monday 9 AM is tomorrow morning, listed once.
        assertEquals(listOf("This evening", "Tomorrow morning"), snoozeShortcuts(at(18, 10)).map { it.title })
    }

    @Test
    fun secondsUntilRoundsUpToTheMinuteWithinU32() {
        val now = 1_000_000L
        assertEquals(60u, secondsUntil(now + 1, now))
        assertEquals(60u, secondsUntil(now + 60_000, now))
        assertEquals(120u, secondsUntil(now + 60_001, now))
        assertNull(secondsUntil(now, now))
        assertNull(secondsUntil(now - 1, now))
        assertEquals(UInt.MAX_VALUE, secondsUntil(now + 200L * 365 * 86_400_000, now))
    }

    @Test
    fun wakeTimeReadsLikeAClock() {
        val now = at(14, 10)
        val us = Locale.US
        assertEquals("6:00 PM", wakeTimeLabel(at(14, 18), now, is24Hour = false, locale = us))
        assertEquals("Thu 9:00 AM", wakeTimeLabel(at(15, 9), now, is24Hour = false, locale = us))
        assertEquals("Thu 09:00", wakeTimeLabel(at(15, 9), now, is24Hour = true, locale = us))
        assertEquals("Oct 26, 9:00 AM", wakeTimeLabel(at(26, 9), now, is24Hour = false, locale = us))
        assertEquals("Jan 4, 2027, 9:00 AM", wakeTimeLabel(ZonedDateTime.of(2027, 1, 4, 9, 0, 0, 0, zone), now, is24Hour = false, locale = us))
    }

    @Test
    fun snoozeUntilIsTheDurationToTheWakeTime() {
        val now = at(14, 10)
        assertEquals(SnoozeOption("until Thu 9:00 AM", "Thu 9:00 AM", 82_800u), snoozeUntil(at(15, 9), now, "Thu 9:00 AM"))
        assertNull(snoozeUntil(at(14, 9), now, "9:00 AM"))
    }

    @Test
    fun shelvesStartCollapsed() {
        assertEquals(emptySet<String>(), openShelves(null))
        assertEquals(emptySet<String>(), openShelves(emptySet()))
        assertEquals(setOf(Shelf.Working.id), openShelves(setOf(Shelf.Working.id)))
    }

    @Test
    fun nextDeadlineIsTheSoonestStillAhead() {
        val now = 1_000_000L
        assertEquals(now + 60_000, nextSnoozeDeadline(listOf(null, now - 5, now + 3_600_000, now + 60_000), now))
        assertNull(nextSnoozeDeadline(listOf(null, now, now - 1), now))
        assertNull(nextSnoozeDeadline(emptyList(), now))
    }

    @Test
    fun captionPrefersTheCoreLabel() {
        val now = 1_000_000L
        assertEquals("in 42m", snoozeCaption("in 42m", now + 1, now))
        assertEquals("in 5m", snoozeCaption(null, now + 5 * 60_000, now))
        // Rounded up: what's left never reads shorter than it is.
        assertEquals("in 1m", snoozeCaption(null, now + 1_000, now))
        assertEquals("in 2h", snoozeCaption(" ", now + 3_600_000 + 1, now))
        assertEquals("in 1h", snoozeCaption(null, now + 59 * 60_000 + 30_000, now))
        assertEquals("in 4h", snoozeCaption(" ", now + 4 * 3_600_000, now))
        assertEquals("in 7d", snoozeCaption(null, now + 7 * 86_400_000L, now))
        assertEquals("now", snoozeCaption(null, now - 1, now))
        assertEquals("later", snoozeCaption(null, null, now))
    }
}
