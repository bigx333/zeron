package sh.zeron.android.core

import uniffi.zeron_core.FrontPage
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.WorkspaceSnapshot

/**
 * How the Sessions page is laid out, kept on this device (the desktop's
 * Default / T3 Mode toggle). T3 is an attention-first inbox with Working,
 * Snoozed and Settled shelves; Settled is the archive under another name.
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

/** A snooze length offered on every touch surface (sheet, swipe, header menu). */
data class SnoozeOption(val label: String, val short: String, val seconds: UInt)

val SnoozeOptions = listOf(
    SnoozeOption("15 minutes", "15m", 15u * 60u),
    SnoozeOption("1 hour", "1h", 60u * 60u),
    SnoozeOption("4 hours", "4h", 4u * 60u * 60u),
    SnoozeOption("1 day", "1d", 24u * 60u * 60u),
    SnoozeOption("1 week", "1w", 7u * 24u * 60u * 60u),
)

/** The T3 shelves; each folds on its own, kept on this device. */
enum class Shelf(val id: String, val title: String) {
    Working("t3.working", "Working"),
    Snoozed("t3.snoozed", "Snoozed"),
    Settled("t3.settled", "Settled"),
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
