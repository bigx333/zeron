package sh.zeron.android.voice

import org.junit.Assert.assertEquals
import org.junit.Test

class DictationTest {
    /** Inserts at the `|` (or replaces the `[…]` selection) and marks where the cursor lands. */
    private fun dictate(draft: String, text: String): String {
        val start = draft.indexOf('[').takeIf { it >= 0 } ?: draft.indexOf('|')
        val clean = draft.replace("[", "").replace("]", "").replace("|", "")
        val end = draft.indexOf(']').takeIf { it >= 0 }?.minus(1) ?: start
        val (next, cursor) = insertingDictation(clean, start, end, text)
        return next.substring(0, cursor) + "|" + next.substring(cursor)
    }

    @Test
    fun emptyDraftTakesTheTextAsIs() {
        assertEquals("Fix the sync test|", dictate("|", "  Fix the sync test "))
    }

    @Test
    fun aSpaceSeparatesTheTextFromWordsOnEitherSide() {
        assertEquals("Please fix the sync test|", dictate("Please|", "fix the sync test"))
        assertEquals("Please fix| the test", dictate("Please|the test", "fix"))
        assertEquals("Please fix| the test", dictate("Please |the test", "fix"))
        assertEquals("Please fix| the test", dictate("Please| the test", "fix"))
    }

    @Test
    fun punctuationAndLineBreaksNeedNoSpace() {
        assertEquals("Fix it|.", dictate("|.", "Fix it"))
        assertEquals("(fix it|)", dictate("(|)", "fix it"))
        assertEquals("First line\nsecond line|", dictate("First line\n|", "second line"))
    }

    @Test
    fun aSelectionIsReplaced() {
        assertEquals("Run the unit| tests", dictate("Run the [flaky] tests", "unit"))
    }

    @Test
    fun noSpeechLeavesTheDraftAlone() {
        assertEquals("Keep| this", dictate("Keep| this", "   "))
    }
}
