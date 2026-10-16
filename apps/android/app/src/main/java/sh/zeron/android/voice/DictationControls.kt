package sh.zeron.android.voice

import android.Manifest
import android.os.SystemClock
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import sh.zeron.android.design.GeistMono
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import sh.zeron.android.ui.ComposerModel
import sh.zeron.android.ui.IconTile
import sh.zeron.android.ui.cardColor
import sh.zeron.android.ui.segmentedShapes
import uniffi.zeron_core.CodexDictationStatus
import uniffi.zeron_core.DeviceView
import java.util.Locale

/** The app's dictation, for composers anywhere in the tree; null leaves dictation out. */
val LocalDictation = staticCompositionLocalOf<DictationController?> { null }

/**
 * Dictation's app-wide side effect, composed once beside [VoiceHost]: the
 * microphone prompt the first dictation asks for. A denial explains itself
 * in a snackbar that opens Settings.
 */
@Composable
fun DictationHost(controller: DictationController) {
    val request by controller.permissionRequest.collectAsState()
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        controller.completePermission(granted)
    }
    LaunchedEffect(request) {
        if (request == null) return@LaunchedEffect
        runCatching { launcher.launch(Manifest.permission.RECORD_AUDIO) }.onFailure { controller.completePermission(false) }
    }
}

/**
 * The composer's microphone, beside send (Codex dictation). Idle: tap to
 * record. Recording: cancel, then a stop pill with the clock that
 * transcribes. Transcribing: cancel beside a loading indicator. The text
 * lands at [model]'s cursor and is never sent. Renders nothing unless
 * dictation is on and the host can lend its ChatGPT credential.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun DictationButton(model: ComposerModel) {
    val controller = LocalDictation.current ?: return
    val ready by controller.ready.collectAsState()
    val phase by controller.phase.collectAsState()
    val call by controller.voice.snapshot.collectAsState()
    val mine = controller.owns(model)
    // A dictation keeps its controls even if the host stops lending mid-way.
    if (!ready && !mine) return
    DisposableEffect(controller, model) {
        controller.attach(model)
        onDispose { controller.detach(model) }
    }
    val haptics = LocalHapticFeedback.current
    val toggle = {
        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
        controller.toggle(model) { text -> model.insertDictation(text) }
    }
    val cancel = {
        haptics.performHapticFeedback(HapticFeedbackType.Reject)
        controller.cancel()
    }
    AnimatedContent(
        if (mine) phase else DictationPhase.Idle,
        transitionSpec = { (scaleIn() + fadeIn()) togetherWith (scaleOut() + fadeOut()) },
        contentKey = { it::class },
        label = "dictation",
    ) { shown ->
        when (shown) {
            DictationPhase.Idle -> IconButton(onClick = toggle, enabled = !call.live && phase == DictationPhase.Idle) {
                ZIcon(ZIcons.Mic, "Dictate", Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            is DictationPhase.Recording -> Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = cancel) { ZIcon(ZIcons.Close, "Cancel dictation", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
                RecordingPill(shown.startedAt, onStop = toggle)
                Spacer(Modifier.width(4.dp))
            }
            DictationPhase.Transcribing -> Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = cancel) { ZIcon(ZIcons.Close, "Cancel dictation", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
                Box(Modifier.size(40.dp).semantics { contentDescription = "Transcribing" }, contentAlignment = Alignment.Center) {
                    LoadingIndicator(Modifier.size(32.dp))
                }
            }
        }
    }
}

/** Recording: a pulsing dot, the clock (up to 1:00) and stop, which transcribes. */
@Composable
private fun RecordingPill(startedAt: Long, onStop: () -> Unit) {
    val seconds by produceState(0L, startedAt) {
        while (true) {
            val elapsed = SystemClock.elapsedRealtime() - startedAt
            value = (elapsed / 1000).coerceIn(0, DictationController.MAX_MILLIS / 1000L)
            delay(1000 - elapsed % 1000)
        }
    }
    val pulse by rememberInfiniteTransition(label = "recording").animateFloat(
        1f,
        0.3f,
        infiniteRepeatable(tween(700), RepeatMode.Reverse),
        label = "dot",
    )
    Surface(
        onClick = onStop,
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.errorContainer,
        contentColor = MaterialTheme.colorScheme.onErrorContainer,
        modifier = Modifier.heightIn(min = 40.dp).semantics { contentDescription = "Stop and transcribe, $seconds seconds recorded" },
    ) {
        Row(
            Modifier.padding(start = 12.dp, end = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(Modifier.size(8.dp).alpha(pulse).clip(CircleShape).background(endRed()))
            Text(
                String.format(Locale.US, "%d:%02d", seconds / 60, seconds % 60),
                fontFamily = GeistMono,
                fontWeight = FontWeight.Medium,
                fontSize = 13.sp,
            )
            ZIcon(ZIcons.Stop, null, Modifier.size(16.dp))
        }
    }
}

/**
 * Settings → Voice → Dictation: the Codex dictation switch, and which
 * device lends ChatGPT (or why none can). Turning it off always works.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
internal fun DictationSetting(controller: DictationController, modifier: Modifier = Modifier) {
    val enabled by controller.enabled.collectAsState()
    val host by controller.host.collectAsState()
    val status by controller.status.collectAsState()
    val devices by controller.voice.devices.collectAsState()
    val selected by controller.voice.selectedHost.collectAsState()
    // The voice host even when it can't lend (an older build), and whether any host is online.
    val candidate = remember(devices, selected) { controller.voice.startHost() }
    val anyHost = remember(devices) { controller.voice.hosts().isNotEmpty() }
    val current = status?.takeIf { it.hostId == host?.id }?.status
    val available = host != null && current?.available == true
    val switchable = enabled || available
    SegmentedListItem(
        onClick = { if (switchable) controller.setEnabled(!enabled) },
        enabled = switchable,
        shapes = segmentedShapes(0, 1),
        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
        leadingContent = { IconTile(ZIcons.Mic) },
        supportingContent = { Text(dictationDetail(host, current, candidate, anyHost)) },
        trailingContent = { Switch(enabled, { controller.setEnabled(it) }, enabled = switchable) },
        modifier = modifier,
    ) { Text("Codex dictation") }
}

private fun dictationDetail(host: DeviceView?, status: CodexDictationStatus?, candidate: DeviceView?, anyHost: Boolean): String = when {
    host != null && status == null -> "Checking ${host.name}…"
    host != null && status?.available == true ->
        "Transcribed by ChatGPT with your Codex account on ${host.name}. Audio goes from this phone to ChatGPT."
    host != null -> status?.message ?: "Codex dictation isn't available on ${host.name}."
    candidate != null -> "Update Zeron on ${candidate.name} to use Codex dictation."
    anyHost -> "Choose the device Codex runs on above."
    else -> "Needs a Mac or server running Zeron, signed in to Codex with ChatGPT."
}

/** Puts dictated [text] at the cursor (replacing a selection), spaced from its neighbours. Never sends. */
fun ComposerModel.insertDictation(text: String) {
    val v = value
    val (next, cursor) = insertingDictation(v.text, v.selection.min, v.selection.max, text)
    value = TextFieldValue(next, TextRange(cursor))
}

/**
 * [draft] with [text] in place of `start..end`, a space on either side
 * where it would otherwise run into a word; the cursor lands after the
 * inserted words.
 */
internal fun insertingDictation(draft: String, start: Int, end: Int, text: String): Pair<String, Int> {
    val words = text.trim()
    val from = start.coerceIn(0, draft.length)
    val until = end.coerceIn(from, draft.length)
    if (words.isEmpty()) return draft to until
    val before = draft.substring(0, from)
    val after = draft.substring(until)
    val lead = if (before.isNotEmpty() && !before.last().isWhitespace() && before.last() !in "([{") " " else ""
    val trail = if (after.isNotEmpty() && !after.first().isWhitespace() && after.first() !in ".,;:!?)]}") " " else ""
    val inserted = lead + words
    return before + inserted + trail + after to before.length + inserted.length
}
