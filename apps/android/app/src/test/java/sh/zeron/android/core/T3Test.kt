package sh.zeron.android.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

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
    }

    @Test
    fun customSnoozeTakesWholePositiveAmounts() {
        assertEquals(SnoozeOption("3 hours", "3h", 10_800u), customSnooze("3", SnoozeUnit.Hours))
        assertEquals(SnoozeOption("1 day", "1d", 86_400u), customSnooze(" 1 ", SnoozeUnit.Days))
        assertEquals(SnoozeOption("1 minute", "1m", 60u), customSnooze("01", SnoozeUnit.Minutes))
        // A week or two is Custom… now that 1w isn't a preset.
        assertEquals(SnoozeOption("14 days", "14d", 1_209_600u), customSnooze("14", SnoozeUnit.Days))
        for (bad in listOf("", "  ", "0", "000", "-1", "+2", "1.5", "1e3", "abc", "3h")) {
            assertNull("'$bad' should not snooze", customSnooze(bad, SnoozeUnit.Hours))
        }
    }

    @Test
    fun customSnoozeFitsTheCoresU32Seconds() {
        // u32 seconds = 4 294 967 295: the largest whole amount of each unit is allowed, one more is not.
        assertEquals(71_582_788L, SnoozeUnit.Minutes.max)
        assertEquals(1_193_046L, SnoozeUnit.Hours.max)
        assertEquals(49_710L, SnoozeUnit.Days.max)
        for (unit in SnoozeUnit.entries) {
            val top = customSnooze("${unit.max}", unit)!!
            assertEquals((unit.max * unit.seconds).toUInt(), top.seconds)
            assertNull(customSnooze("${unit.max + 1}", unit))
        }
        assertNull(customSnooze("99999999999999999999", SnoozeUnit.Minutes))
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
