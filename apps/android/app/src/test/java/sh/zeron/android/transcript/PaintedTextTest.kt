package sh.zeron.android.transcript

import org.junit.Assert.assertEquals
import org.junit.Test

class PaintedTextTest {
    @Test
    fun separatePiecesOfALinkAreSpaced() {
        // A delegation line: "Delegated" then the task's title, laid out
        // side by side with a gap; the row text runs them together.
        val runs = listOf(
            PaintedRun("Delegated", 0, 20f, 60f, 14f),
            PaintedRun("Audit the budget", 9, 88f, 110f, 14f),
        )
        assertEquals("Delegated Audit the budget", joinPainted(runs))
    }

    @Test
    fun runsThatContinueEachOtherJoinAsWritten() {
        // A pill's title shaped as two runs (a fallback font mid-word, say).
        val runs = listOf(
            PaintedRun("Revi", 4, 30f, 24f, 14f),
            PaintedRun("éw notes", 8, 54f, 50f, 14f),
        )
        assertEquals("Reviéw notes", joinPainted(runs))
    }

    @Test
    fun whitespaceCollapsesAndUtf16OffsetsHold() {
        // An emoji is two UTF-16 units: the next run still continues it.
        val runs = listOf(
            PaintedRun("  🚀", 0, 10f, 20f, 14f),
            PaintedRun("Ship  it ", 4, 30f, 40f, 14f),
            PaintedRun("again", 13, 0f, 30f, 32f),
        )
        assertEquals("🚀Ship it again", joinPainted(runs))
    }
}
