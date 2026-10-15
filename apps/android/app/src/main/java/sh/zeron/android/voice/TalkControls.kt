package sh.zeron.android.voice

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.R
import sh.zeron.android.design.ZIcon
import sh.zeron.android.ui.successColor
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

/** The labelled primary action, docked above the centre of the PTT call bar. */
@Composable
internal fun TalkDisc(controller: VoiceController, snapshot: VoiceSnapshot, modifier: Modifier = Modifier) {
    val enabled = snapshot.live && snapshot.active
    val held = enabled && !snapshot.muted
    val reducedMotion = rememberReducedMotion()
    val scale by animateFloatAsState(
        if (held && !reducedMotion) 1.06f else 1f,
        if (reducedMotion) snap() else spring(dampingRatio = 0.6f, stiffness = 500f),
        label = "talk-disc-scale",
    )
    val level by animateFloatAsState(
        if (held && !reducedMotion) snapshot.state?.microphone?.coerceIn(0f, 1f) ?: 0f else 0f,
        if (reducedMotion) snap() else tween(80), label = "talk-disc-level",
    )
    val breath = if (enabled && !held && !reducedMotion) {
        val transition = rememberInfiniteTransition(label = "talk-disc-breath")
        val opacity by transition.animateFloat(0.3f, 0.55f, infiniteRepeatable(tween(2400), RepeatMode.Reverse), label = "talk-disc-dots")
        opacity
    } else 0.45f
    val ink = MaterialTheme.colorScheme.onSurface
    val background = MaterialTheme.colorScheme.background
    val success = successColor()
    val fill = if (held) ink else MaterialTheme.colorScheme.surfaceContainerHighest
    val content = if (held) MaterialTheme.colorScheme.background else ink
    val fontScale = LocalDensity.current.fontScale
    Box(
        modifier.size(132.dp)
            .semantics(mergeDescendants = true) { contentDescription = "Hold to talk" }
            .holdToTalk(controller, enabled, snapshot.muted)
            .clickable(enabled = enabled, role = Role.Button, onClickLabel = "Touch and hold to talk") { }
            .alpha(if (enabled) 1f else 0.42f),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(132.dp).graphicsLayer { scaleX = scale; scaleY = scale }) {
            if (held) {
                drawCircle(Brush.radialGradient(listOf(success.copy(alpha = 0.23f), Color.Transparent), center, 76.dp.toPx()), 76.dp.toPx())
            }
            drawCircle(background, 54.dp.toPx())
            if (!enabled) return@Canvas
            repeat(44) { i ->
                val angle = i * 2 * PI / 44
                val radius = (58f + if (held) level * 7f * (0.6f + 0.4f * sin(angle * 3).toFloat()) else 0f).dp.toPx()
                drawCircle(
                    if (held) success else ink.copy(alpha = breath), 1.6.dp.toPx(),
                    Offset(center.x + cos(angle).toFloat() * radius, center.y + sin(angle).toFloat() * radius),
                )
            }
        }
        Column(
            Modifier.size(96.dp).graphicsLayer { scaleX = scale; scaleY = scale }
                .background(fill, CircleShape)
                .border(1.dp, if (held) success else ink.copy(alpha = 0.14f), CircleShape),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(5.dp, Alignment.CenterVertically),
        ) {
            ZIcon(R.drawable.zi_microphone, null, Modifier.size(28.dp), tint = content)
            Text(
                if (!enabled) "Connecting" else if (held) "Talking" else "Hold to talk",
                color = content, fontSize = (11.5f * fontScale.coerceAtMost(1.3f) / fontScale).sp,
                fontWeight = FontWeight.SemiBold, maxLines = 1,
            )
        }
    }
}

/** A word stays recognisable when the ongoing call is reduced to its strip. */
@Composable
internal fun TalkPill(controller: VoiceController, snapshot: VoiceSnapshot, modifier: Modifier = Modifier) {
    val enabled = snapshot.live && snapshot.active
    val held = enabled && !snapshot.muted
    val ink = MaterialTheme.colorScheme.onSurface
    val fill = if (held) ink else MaterialTheme.colorScheme.surfaceContainerHighest
    val content = if (held) MaterialTheme.colorScheme.background else ink
    Box(
        modifier.size(78.dp, 48.dp)
            .semantics(mergeDescendants = true) { contentDescription = "Hold to talk" }
            .holdToTalk(controller, enabled, snapshot.muted)
            .clickable(enabled = enabled, role = Role.Button, onClickLabel = "Touch and hold to talk") { }
            .alpha(if (enabled) 1f else 0.42f),
        contentAlignment = Alignment.Center,
    ) {
        Row(
            Modifier.size(78.dp, 40.dp).background(fill, RoundedCornerShape(50))
                .border(if (held) 1.5.dp else 1.dp, if (held) successColor() else ink.copy(alpha = 0.16f), RoundedCornerShape(50)),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally),
        ) {
            ZIcon(R.drawable.zi_microphone, null, Modifier.size(16.dp), tint = content)
            Text("Talk", color = content, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1)
        }
    }
}
