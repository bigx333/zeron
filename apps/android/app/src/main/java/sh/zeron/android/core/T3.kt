package sh.zeron.android.core

import java.time.DayOfWeek
import java.time.Duration
import java.time.LocalDate
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.temporal.TemporalAdjusters
import java.util.Locale
import uniffi.zeron_core.FrontPage
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.WorkspaceSnapshot

/**
 * How the Sessions page is laid out, kept on this device (the desktop's
 * Default / T3 Mode toggle). T3 is an attention-first inbox with Working
 * and Snoozed shelves; Settled is the archive under another name, kept off
 * the page like Archived.
 */
enum class SidebarMode {
    Default,
    T3;

    val isT3 get() = this == T3

    /** What archiving is called in this mode. */
    val settle get() = if (isT3) "Settle" else "Archive"
    val unsettle get() = if (isT3) "Unsettle" else "Unarchive"
    val settled get() = if (isT3) "Settled" else "Archived"

    companion object {
        fun parse(value: String?): SidebarMode = entries.firstOrNull { it.name == value } ?: Default
    }
}

/**
 * A snooze, as the core takes it (seconds from now). [phrase] finishes
 * "Snooze …" / "Snoozed …": "for 1 hour", "until Thu 9:00 AM".
 */
data class SnoozeOption(val phrase: String, val short: String, val seconds: UInt)

/** The presets; past them, ⋯ asks when it should come back ([snoozeShortcuts], or a picked date and time). */
val SnoozeOptions = listOf(
    SnoozeOption("for 15 minutes", "15m", 15u * 60u),
    SnoozeOption("for 1 hour", "1h", 60u * 60u),
    SnoozeOption("for 4 hours", "4h", 4u * 60u * 60u),
    SnoozeOption("for 1 day", "1d", 24u * 60u * 60u),
)

/** A "Snooze until" shortcut in local time. */
data class SnoozeShortcut(val title: String, val at: ZonedDateTime)

/**
 * The local-time shortcuts under ⋯: This evening (6 PM, while that's over an
 * hour away), Tomorrow morning (9 AM), and Next week (Monday 9 AM, unless
 * that's tomorrow).
 */
fun snoozeShortcuts(now: ZonedDateTime): List<SnoozeShortcut> {
    val today = now.toLocalDate()
    fun at(date: LocalDate, hour: Int) = date.atTime(hour, 0).atZone(now.zone)
    val evening = at(today, 18)
    val tomorrow = at(today.plusDays(1), 9)
    val nextWeek = at(today.with(TemporalAdjusters.next(DayOfWeek.MONDAY)), 9)
    return buildList {
        if (Duration.between(now, evening) > Duration.ofHours(1)) add(SnoozeShortcut("This evening", evening))
        add(SnoozeShortcut("Tomorrow morning", tomorrow))
        if (nextWeek != tomorrow) add(SnoozeShortcut("Next week", nextWeek))
    }
}

/**
 * Seconds from now until [wakeMs], for the core's duration API: rounded up to
 * the minute, at least a minute, at most u32. Null once it's not ahead.
 */
fun secondsUntil(wakeMs: Long, nowMs: Long): UInt? {
    val left = wakeMs - nowMs
    if (left <= 0) return null
    val seconds = (left + 59_999) / 60_000 * 60
    return seconds.coerceIn(60, UInt.MAX_VALUE.toLong()).toUInt()
}

/** "6:00 PM" today, "Thu 9:00 AM" within the week, "Oct 22, 9:00 AM" later, with the year once it isn't this one. */
fun wakeTimeLabel(at: ZonedDateTime, now: ZonedDateTime, is24Hour: Boolean, locale: Locale = Locale.getDefault()): String {
    val time = if (is24Hour) "HH:mm" else "h:mm a"
    val days = Duration.between(now.toLocalDate().atStartOfDay(), at.toLocalDate().atStartOfDay()).toDays()
    val pattern = when {
        days == 0L -> time
        days in 1..6 -> "EEE $time"
        at.year == now.year -> "MMM d, $time"
        else -> "MMM d, yyyy, $time"
    }
    return at.format(DateTimeFormatter.ofPattern(pattern, locale))
}

/** Snooze until [at] (labelled [label]), or null when that's not ahead of [now]. */
fun snoozeUntil(at: ZonedDateTime, now: ZonedDateTime, label: String): SnoozeOption? =
    secondsUntil(at.toInstant().toEpochMilli(), now.toInstant().toEpochMilli())?.let { SnoozeOption("until $label", label, it) }

/** The T3 shelves; each folds on its own, kept on this device. */
enum class Shelf(val id: String, val title: String) {
    Working("t3.working", "Working"),
    Snoozed("t3.snoozed", "Snoozed"),
}

/** Which shelves are open, from the stored set (null before the user ever opened one): all start collapsed. */
fun openShelves(stored: Set<String>?): Set<String> = stored ?: emptySet()

/** The front page this mode renders. */
fun WorkspaceSnapshot.frontFor(mode: SidebarMode): FrontPage = if (mode.isT3) t3Front else front

/** Every active row on a front page once (pins, sections, recent, then Working), for filters and counts. */
fun FrontPage.activeRows(): List<SessionRow> {
    val seen = HashSet<String>()
    return (pinned + sections.flatMap { it.sessions } + recent + working).filter { seen.add(it.id) }
}

/** The core's "in 42m", or the same wording from the deadline when it sent none (rounded up, like the core's). */
fun snoozeCaption(label: String?, untilMs: Long?, nowMs: Long = System.currentTimeMillis()): String {
    label?.takeIf { it.isNotBlank() }?.let { return it }
    untilMs ?: return "later"
    val left = untilMs - nowMs
    if (left <= 0) return "now"
    fun up(unit: Long) = (left + unit - 1) / unit
    return when {
        up(60_000) < 60 -> "in ${up(60_000)}m"
        up(3_600_000) < 24 -> "in ${up(3_600_000)}h"
        else -> "in ${up(86_400_000)}d"
    }
}

/** The soonest snooze deadline still ahead, so the page re-reads when a row wakes. */
fun nextSnoozeDeadline(deadlines: List<Long?>, nowMs: Long): Long? =
    deadlines.filterNotNull().filter { it > nowMs }.minOrNull()
