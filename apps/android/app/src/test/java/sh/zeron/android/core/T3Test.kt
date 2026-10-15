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
        assertEquals(listOf("15m", "1h", "4h", "1d", "1w"), SnoozeOptions.map { it.short })
        assertEquals(listOf(900u, 3_600u, 14_400u, 86_400u, 604_800u), SnoozeOptions.map { it.seconds })
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
