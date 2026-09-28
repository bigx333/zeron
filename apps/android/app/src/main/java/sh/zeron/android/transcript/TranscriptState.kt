package sh.zeron.android.transcript

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import sh.zeron.android.core.TextEngine
import uniffi.zeron_core.LayoutFrame
import uniffi.zeron_core.LayoutListener
import uniffi.zeron_core.RowKind
import uniffi.zeron_core.TranscriptView
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.max

/**
 * One transcript's layout engine plus the viewport over it. Rust owns
 * geometry: each [LayoutFrame] gives exact row offsets, so this only tracks
 * the scroll offset (dp) and keeps the user's place across frames — anchored
 * to a row, or following the tail.
 */
@Stable
class TranscriptState {
    private val main = Handler(Looper.getMainLooper())
    private val pending = AtomicBoolean(false)

    val engine: TranscriptView = TranscriptView(TextEngine.shared, object : LayoutListener {
        // Layout thread: coalesce to one pull per main-loop turn.
        override fun frameReady(revision: ULong) {
            if (pending.compareAndSet(false, true)) main.post {
                pending.set(false)
                if (!closed) apply(engine.frame())
            }
        }
    })

    var frame by mutableStateOf<LayoutFrame?>(null)
        private set
    /** Scroll offset in dp (0 = top of the content). */
    var offset by mutableFloatStateOf(0f)
    var viewport by mutableFloatStateOf(0f)
    /** Space the composer covers at the bottom (dp). */
    var bottomInset by mutableFloatStateOf(0f)
    var following by mutableStateOf(true)
    var dragging by mutableStateOf(false)
    val fonts = StyleFonts()
    /** Rows present before a frame arrived don't fade in. */
    internal val knownKeys = HashSet<ULong>()
    internal var settled = false

    private val cache = object : LinkedHashMap<Triple<ULong, ULong, Float>, RowModel>(64, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Triple<ULong, ULong, Float>, RowModel>?) = size > 400
    }

    private var closed = false
    private var width = 0f
    private var scale = 0f

    val contentHeight: Float get() = (frame?.totalHeight() ?: 0f) + bottomInset
    val maxOffset: Float get() = max(0f, contentHeight - viewport)
    val distanceFromBottom: Float get() = maxOffset - offset

    fun setViewport(widthDp: Float, heightDp: Float, textScale: Float) {
        viewport = heightDp
        if (widthDp != width || textScale != scale) {
            width = widthDp
            scale = textScale
            engine.setViewport(widthDp, textScale)
        }
    }

    fun apply(new: LayoutFrame) {
        val old = frame
        val first = old == null || old.rowCount() == 0u
        var anchor: Pair<ULong, Float>? = null
        if (!following && old != null) {
            old.indexAt(max(0f, offset))?.let { i ->
                old.placement(i)?.let { p -> anchor = p.key to (offset - p.y) }
            }
        }
        frame = new
        if (first && new.rowCount() > 0u) {
            for (i in 0u until new.rowCount()) new.placement(i)?.let { knownKeys.add(it.key) }
            offset = maxOffset
            settled = true
        } else if (!following) {
            anchor?.let { (key, delta) ->
                new.indexOf(key)?.let { i -> new.placement(i)?.let { p -> offset = (p.y + delta).coerceIn(0f, maxOffset) } }
            }
        }
    }

    /** The display model for a row at the frame's width (cached per version). */
    fun model(frame: LayoutFrame, index: UInt, key: ULong, version: ULong): RowModel? {
        val k = Triple(key, version, frame.width())
        cache[k]?.let { return it }
        val display = frame.display(index) ?: return null
        return RowModel(display).also { cache[k] = it }
    }

    /** User scroll by `delta` dp (positive = toward the tail). Returns the consumed amount. */
    fun scrollBy(delta: Float): Float {
        val before = offset
        offset = (offset + delta).coerceIn(0f, maxOffset)
        // Momentum carrying the list back into the last 70dp re-latches follow.
        if (!following && !dragging && delta > 0 && distanceFromBottom < 70f) following = true
        return offset - before
    }

    fun scrollToBottom() {
        following = true
    }

    fun toggle(key: ULong) = engine.toggle(key)
    fun toggleDetail(row: ULong, detail: ULong, open: Boolean) = engine.toggleDetail(row, detail, open)

    /** Newest user row keys (the runway looks for a new one after a send). */
    fun recentUserKeys(): Set<ULong> {
        val f = frame ?: return emptySet()
        val n = f.rowCount().toInt()
        val out = HashSet<ULong>()
        for (i in (n - 1) downTo max(0, n - 24)) f.placement(i.toUInt())?.let { if (it.kind == RowKind.USER) out.add(it.key) }
        return out
    }

    fun close() {
        closed = true
        engine.shutdown()
        engine.close()
    }
}
