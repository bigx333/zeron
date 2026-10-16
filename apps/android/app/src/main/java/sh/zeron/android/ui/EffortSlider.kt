package sh.zeron.android.ui

import android.provider.Settings
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.RestartAlt
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.reasoningLabel
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.random.Random

/** The top tiers (Codex's Ultra, Claude's Ultracode and Ultrathink). */
private fun peak(level: String) = level == "ultra" || level == "ultracode" || level == "ultrathink"

/**
 * Effort as heat: calm sky at the low end, through blue, indigo and violet,
 * to fuchsia at the top. The fill shows the band of it that ends at the
 * level, so the whole track warms as the effort rises.
 */
private val Heat = listOf(
    0f to Color(0xFF38BDF8),
    0.35f to Color(0xFF3B82F6),
    0.6f to Color(0xFF6366F1),
    0.82f to Color(0xFF8B5CF6),
    1f to Color(0xFFD946EF),
)

private fun heat(at: Float): Color {
    val f = at.coerceIn(0f, 1f)
    val i = Heat.indexOfFirst { it.first >= f }.coerceAtLeast(1)
    val (a, ca) = Heat[i - 1]
    val (b, cb) = Heat[i]
    return lerp(ca, cb, if (b > a) (f - a) / (b - a) else 1f)
}

/** 0 below [from], 1 above [to], eased between. */
private fun ramp(from: Float, to: Float, x: Float): Float {
    val t = ((x - from) / (to - from)).coerceIn(0f, 1f)
    return t * t * (3 - 2 * t)
}

/** One drifting point of light in the filled track. Positions are fractions of the track; speed scales the drift. */
private class Spark(val x: Float, val y: Float, val radius: Float, val speed: Float, val phase: Float, val twinkle: Float)

private val Sparks = Random(7).let { r ->
    List(44) { Spark(r.nextFloat(), 0.18f + 0.64f * r.nextFloat(), 0.8f + 1.4f * r.nextFloat(), 0.5f + r.nextFloat(), r.nextFloat(), 1f + 2f * r.nextFloat()) }
}

/**
 * The slider's motion, advanced every frame at a pace set by the effort, so
 * a change of level speeds it up or calms it down without a jump.
 */
private class Motion {
    /** Drift of the sparks, in track widths. */
    var drift by mutableFloatStateOf(0f)
    /** The sheen's sweep across the fill, 0–1. */
    var sweep by mutableFloatStateOf(0f)
    /** A slow breath for the glow and halo, in cycles. */
    var breath by mutableFloatStateOf(0f)

    fun advance(seconds: Float, intensity: Float) {
        drift += seconds * (0.03f + 0.17f * intensity)
        sweep = (sweep + seconds / (3.4f - 2.2f * intensity)) % 1f
        breath += seconds * (0.4f + 0.6f * intensity)
    }
}

/**
 * The model's effort levels on a stepped track, low to high, named above it
 * in the level's color. Effort reads as heat building up: the fill warms from
 * sky to fuchsia, a sheen sweeps it, sparks thicken and quicken (streaking at
 * the high end), a glow breathes around the track from Extra High up, and the
 * top tiers pulse a halo from the thumb. The thumb springs to each step with
 * a tick (a firmer one into the top tiers); the choice applies when it
 * settles (tap or release). Reset returns to the model's default. With
 * system animations off it holds still.
 */
@Composable
fun EffortSlider(
    effort: String,
    efforts: List<String>,
    default: String?,
    onEffort: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = LocalHapticFeedback.current
    val context = LocalContext.current
    val last = (efforts.size - 1).coerceAtLeast(1)
    var step by remember(efforts) { mutableIntStateOf(efforts.indexOf(effort).coerceAtLeast(0)) }
    // A change from elsewhere (another device, a model switch) moves the thumb.
    LaunchedEffect(effort, efforts) { step = efforts.indexOf(effort).coerceAtLeast(0) }
    // The finger's position while dragging; the thumb springs to the step otherwise.
    var dragging by remember { mutableStateOf<Float?>(null) }
    val settled by animateFloatAsState(
        step / last.toFloat(),
        spring(dampingRatio = 0.75f, stiffness = Spring.StiffnessMediumLow),
        label = "effort thumb",
    )
    val fraction = dragging ?: settled
    val level = efforts.getOrElse(step) { effort }
    val top by animateFloatAsState(if (peak(level)) 1f else 0f, tween(500), label = "effort peak")
    // The level's color for the label and icon; never lighter than blue, so it reads on white.
    val accent = heat(0.35f + 0.65f * fraction)

    val motion = remember { Motion() }
    val intensity by rememberUpdatedState(fraction)
    val still = remember { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
    LaunchedEffect(still) {
        if (still) return@LaunchedEffect
        var then = withFrameNanos { it }
        while (true) {
            withFrameNanos { now ->
                motion.advance((now - then) / 1e9f, intensity)
                then = now
            }
        }
    }

    val track = MaterialTheme.colorScheme.surfaceContainerHighest
    val tick = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.45f)

    fun pick(to: Int) {
        val s = to.coerceIn(0, efforts.size - 1)
        if (s == step) return
        val rising = !peak(efforts[step]) && peak(efforts[s])
        step = s
        haptics.performHapticFeedback(if (rising) HapticFeedbackType.Confirm else HapticFeedbackType.SegmentTick)
    }
    fun commit() = onEffort(efforts[step])

    Column(modifier) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 40.dp)) {
            ZIcon(ZIcons.Effort, null, Modifier.size(20.dp), tint = accent)
            Spacer(Modifier.width(10.dp))
            AnimatedContent(
                reasoningLabel(level),
                transitionSpec = { fadeIn(tween(180)) togetherWith fadeOut(tween(120)) using SizeTransform(clip = false) },
                label = "effort label",
                modifier = Modifier.weight(1f),
            ) { Text(it, style = MaterialTheme.typography.titleMedium, color = accent) }
            AnimatedVisibility(default != null && level != default, enter = fadeIn() + scaleIn(), exit = fadeOut() + scaleOut()) {
                IconButton(onClick = {
                    val to = efforts.indexOf(default)
                    if (to >= 0) {
                        pick(to)
                        commit()
                    }
                }) {
                    Icon(
                        Icons.Rounded.RestartAlt,
                        "Reset to the model's default",
                        Modifier.size(22.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        Spacer(Modifier.height(10.dp))
        Canvas(
            Modifier
                .fillMaxWidth()
                .height(44.dp)
                .semantics {
                    contentDescription = "Effort"
                    stateDescription = reasoningLabel(level)
                    setProgress { value ->
                        pick((value * last).roundToInt())
                        commit()
                        true
                    }
                }
                .pointerInput(efforts) {
                    val r = size.height / 2f
                    fun at(x: Float) = ((x - r) / (size.width - 2 * r)).coerceIn(0f, 1f)
                    detectTapGestures { o ->
                        pick((at(o.x) * last).roundToInt())
                        commit()
                    }
                }
                .pointerInput(efforts) {
                    val r = size.height / 2f
                    fun at(x: Float) = ((x - r) / (size.width - 2 * r)).coerceIn(0f, 1f)
                    detectHorizontalDragGestures(
                        onDragStart = { o -> dragging = at(o.x) },
                        onDragEnd = {
                            dragging = null
                            commit()
                        },
                        onDragCancel = { dragging = null },
                    ) { change, _ ->
                        change.consume()
                        val f = at(change.position.x)
                        dragging = f
                        pick((f * last).roundToInt())
                    }
                },
        ) {
            val r = size.height / 2f
            val run = size.width - 2 * r
            val thumbX = r + fraction * run
            val fillWidth = thumbX + r
            val hot = heat(fraction)
            val breath = (0.5f + 0.5f * sin(2 * PI * motion.breath).toFloat())

            // A glow around the track from Extra High up, breathing; strongest in the top tiers.
            val glow = ramp(0.55f, 1f, fraction) * (0.55f + 0.45f * breath) * (0.7f + 0.3f * top)
            if (glow > 0.01f) {
                for (k in 1..5) {
                    val grow = k * 2.5f * density
                    drawRoundRect(
                        hot,
                        topLeft = Offset(-grow, -grow),
                        size = Size(fillWidth + 2 * grow, size.height + 2 * grow),
                        cornerRadius = CornerRadius(r + grow, r + grow),
                        alpha = glow * 0.16f / k,
                    )
                }
            }

            drawRoundRect(track, cornerRadius = CornerRadius(r, r))
            // Steps not yet reached, as dots in the empty track.
            for (i in 0..last) {
                val x = r + i / last.toFloat() * run
                if (x > thumbX + r) drawCircle(tick, radius = 2.5.dp.toPx(), center = Offset(x, r))
            }

            val fill = Path().apply { addRoundRect(RoundRect(0f, 0f, fillWidth, size.height, CornerRadius(r, r))) }
            clipPath(fill) {
                // A band of the spectrum ending at the level's heat: calm sky at Low,
                // indigo to fuchsia end to end at the top.
                val from = (fraction - 0.5f).coerceAtLeast(0f)
                val stops = (0..6).map { i -> i / 6f to heat(from + (fraction - from) * i / 6f) }.toTypedArray()
                drawRect(Brush.horizontalGradient(*stops, startX = 0f, endX = fillWidth))

                // A sheen sweeping through, quicker and brighter with the effort.
                val band = size.height * 2.2f
                val x = -band + motion.sweep * (fillWidth + 2 * band)
                drawRect(
                    Brush.horizontalGradient(
                        0f to Color.Transparent,
                        0.5f to Color.White.copy(alpha = 0.10f + 0.22f * fraction),
                        1f to Color.Transparent,
                        startX = x,
                        endX = x + band,
                    ),
                )

                // Sparks: denser and quicker with the effort, streaking at the high end.
                val shown = (8 + (Sparks.size - 8) * fraction).toInt()
                val streak = ramp(0.6f, 1f, fraction) * 14.dp.toPx()
                for (s in Sparks.take(shown)) {
                    val sx = ((s.x + motion.drift * s.speed) % 1f) * fillWidth
                    val sy = s.y * size.height
                    val twinkle = abs(sin(2 * PI * (motion.breath * s.twinkle + s.phase))).toFloat()
                    val alpha = (0.3f + 0.6f * twinkle) * (0.7f + 0.3f * fraction)
                    if (streak > 1f) {
                        drawLine(
                            Brush.horizontalGradient(0f to Color.Transparent, 1f to Color.White.copy(alpha = alpha * 0.6f), startX = sx - streak * s.speed, endX = sx),
                            Offset(sx - streak * s.speed, sy),
                            Offset(sx, sy),
                            strokeWidth = s.radius.dp.toPx(),
                            cap = StrokeCap.Round,
                        )
                    }
                    drawCircle(Color.White, radius = s.radius.dp.toPx(), center = Offset(sx, sy), alpha = alpha)
                }
            }

            // The top tiers: a halo pulsing out from the thumb.
            if (top > 0.01f) {
                val wave = (motion.breath * 1.5f) % 1f
                drawCircle(
                    hot,
                    radius = r - 4.dp.toPx() + wave * 10.dp.toPx(),
                    center = Offset(thumbX, r),
                    alpha = top * 0.6f * (1 - wave),
                    style = Stroke(2.dp.toPx()),
                )
            }
            // The thumb: white, inset in the fill, with a soft edge.
            drawCircle(Color.Black.copy(alpha = 0.18f), radius = r - 3.dp.toPx(), center = Offset(thumbX, r + 1.dp.toPx()))
            drawCircle(Color.White, radius = r - 4.dp.toPx(), center = Offset(thumbX, r))
        }
    }
}
