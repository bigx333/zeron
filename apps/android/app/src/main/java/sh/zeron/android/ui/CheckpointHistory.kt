package sh.zeron.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import sh.zeron.android.R
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.ChatIndicator
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.RunCheckpointView
import uniffi.zeron_core.SessionRow

// History in the tasks sheet: the checkpoints each finished turn leaves
// (conversation, provider session and, when captured, the checkout), with
// restore in place. A restore keeps where you were as a recovery point, so
// the section offers Undo right after one and keeps recovery points folded
// for inspection. Every word comes from what the core returned.

/** What the provider does on restore, in plain words. */
internal fun strategyText(strategy: String): String? = when (strategy) {
    "native" -> "Resumes the provider session at this point."
    "portable" -> "Continues from a copy of the conversation."
    else -> null
}

/** What a restore does, said before it happens. */
internal fun restoreBlurb(point: RunCheckpointView): String = when {
    point.recovery && point.filesAvailable -> "The conversation and the checkout return to where they were before the restore. Where you are now is saved as a recovery point."
    point.recovery -> "The conversation returns to where it was before the restore. Where you are now is saved as a recovery point."
    point.filesAvailable -> "The conversation and the checkout go back to just after this turn. Where you are now is saved as a recovery point."
    else -> "The conversation goes back to just after this turn; no files were saved here, so the checkout stays as it is. Where you are now is saved as a recovery point."
}

/** Why Restore can't run right now, the core's word last. `sharedTasks`: a task still works in this checkout. */
internal fun restoreBlocked(point: RunCheckpointView, chat: SessionRow, sharedTasks: Boolean): String? = when {
    !chat.deviceOnline -> "Its device is offline."
    chat.indicator == ChatIndicator.WORKING || chat.indicator == ChatIndicator.AWAITING_INPUT -> "Finish or stop the current turn first."
    sharedTasks -> "Stop tasks working in this checkout first."
    !point.restorable -> point.blockedReason ?: "This point can't be restored."
    else -> null
}

private fun filesFact(point: RunCheckpointView): String = when {
    !point.filesAvailable -> "Conversation only"
    point.filesChanged == null -> "Checkout saved"
    point.filesChanged == 0u -> "Checkout unchanged since"
    point.filesChanged == 1u -> "1 file differs"
    else -> "${point.filesChanged} files differ"
}

private fun ago(ms: Long): String =
    uniffi.zeron_core.relativeTimeLabel(atMs = ms, nowMs = System.currentTimeMillis()).let { if (it == "now") "just now" else "$it ago" }

private sealed interface HistoryLoad {
    data object Loading : HistoryLoad
    data class Ready(val points: List<RunCheckpointView>) : HistoryLoad
    data class Failed(val message: String) : HistoryLoad
    /** The host predates checkpoints: the section stays away. */
    data object Unsupported : HistoryLoad
}

/** `at`: when the restore landed; a turn finished after it retires Undo. */
private data class Receipt(val title: String, val recovery: String, val undone: Boolean = false, val at: Long = System.currentTimeMillis())

/**
 * The History section. Reads again after each finished turn (a new checkpoint
 * is saved then), after a save, and after a restore.
 */
@Composable
fun CheckpointHistory(core: CoreClient, chat: SessionRow, children: List<SessionRow>, reduced: Boolean) {
    var reload by remember(chat.id) { mutableIntStateOf(0) }
    var load by remember(chat.id) { mutableStateOf<HistoryLoad>(HistoryLoad.Loading) }
    var receipt by remember(chat.id) { mutableStateOf<Receipt?>(null) }
    var restoring by remember(chat.id) { mutableStateOf<String?>(null) }
    var confirm by remember(chat.id) { mutableStateOf<RunCheckpointView?>(null) }
    val errors = remember(chat.id) { mutableStateMapOf<String, String>() }
    val requests = remember(chat.id) { mutableMapOf<String, String>() }
    var saving by remember(chat.id) { mutableStateOf(false) }
    var saveError by remember(chat.id) { mutableStateOf<String?>(null) }
    var recoveryOpen by remember(chat.id) { mutableStateOf(false) }
    var shown by remember(chat.id) { mutableIntStateOf(5) }
    val scope = rememberCoroutineScope()

    // Checkpoints land at turn end: while a turn runs its activity ticks
    // don't re-read (the key holds), the read after it finishes does.
    val busy = chat.indicator == ChatIndicator.WORKING || chat.indicator == ChatIndicator.AWAITING_INPUT
    var readKey by remember(chat.id) { mutableLongStateOf(chat.lastActivityMs) }
    LaunchedEffect(busy, chat.lastActivityMs) { if (!busy) readKey = chat.lastActivityMs }
    val sharedTasks = children.any { row -> row.task?.let { !it.terminal && it.isolation == "shared" } == true }
    LaunchedEffect(chat.id, readKey, reload) {
        load = try {
            val points = core.chatCheckpoints(chatId = chat.id)
            // The chat moved on after the restore: Undo would now mean something else.
            receipt?.let { shown -> if (points.any { !it.recovery && it.createdAtMs > shown.at }) receipt = null }
            HistoryLoad.Ready(points)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: CoreException.Unsupported) {
            HistoryLoad.Unsupported
        } catch (_: CoreException.NotImplemented) {
            HistoryLoad.Unsupported
        } catch (failure: Exception) {
            if (load is HistoryLoad.Ready) load else HistoryLoad.Failed(failure.message.orEmpty())
        }
    }

    fun rollback(id: String, title: String, undo: Boolean) {
        if (restoring != null) return
        restoring = id; errors.remove(id)
        val request = requests.getOrPut(id) { UUID.randomUUID().toString() }
        scope.launch {
            try {
                val recovery = core.rollbackChat(chatId = chat.id, checkpointId = id, clientRequestId = request)
                requests.remove(id)
                receipt = if (undo) receipt?.copy(recovery = recovery, undone = true, at = System.currentTimeMillis()) else Receipt(title, recovery)
                reload++
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                errors[id] = "Couldn't restore. ${failure.message.orEmpty()}"
            } finally {
                restoring = null
            }
        }
    }

    if (load is HistoryLoad.Unsupported) return
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.fillMaxWidth().padding(top = 10.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("History", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            TextButton(
                onClick = {
                    saving = true; saveError = null
                    scope.launch {
                        try { core.createChatCheckpoint(chatId = chat.id); reload++ }
                        catch (cancelled: CancellationException) { throw cancelled }
                        catch (failure: Exception) { saveError = "Couldn't save a checkpoint. ${failure.message.orEmpty()}" }
                        finally { saving = false }
                    }
                },
                enabled = chat.deviceOnline && !saving,
                modifier = Modifier.heightIn(min = 48.dp),
            ) {
                if (saving) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp) else ZIcon(ZIcons.Plus, null, Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp)); Text(if (saving) "Saving…" else "Save now")
            }
        }
        saveError?.let { HistoryError(it) }

        // The last restore's receipt, easing in once; Undo restores the
        // recovery point it kept (which keeps one of its own).
        AnimatedVisibility(
            visible = receipt != null,
            enter = if (reduced) fadeIn(tween(0)) else fadeIn(tween(180)) + expandVertically(tween(220)),
            exit = if (reduced) fadeOut(tween(0)) else fadeOut(tween(120)) + shrinkVertically(tween(160)),
        ) {
            val shownReceipt = receipt ?: return@AnimatedVisibility
            Column(
                Modifier.fillMaxWidth().clip(MaterialTheme.shapes.medium)
                    .background(successColor().copy(alpha = 0.10f)).padding(horizontal = 12.dp, vertical = 6.dp)
                    .semantics { liveRegion = LiveRegionMode.Polite },
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ZIcon(ZIcons.Check, null, Modifier.size(16.dp), successColor())
                    Spacer(Modifier.width(8.dp))
                    Text(
                        if (shownReceipt.undone) "Undone. The restore is kept as a recovery point." else "Restored to “${shownReceipt.title}”.",
                        style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f),
                    )
                    if (!shownReceipt.undone) {
                        TextButton(
                            onClick = { rollback(shownReceipt.recovery, shownReceipt.title, undo = true) },
                            enabled = restoring == null,
                            modifier = Modifier.heightIn(min = 48.dp),
                        ) { Text(if (restoring == shownReceipt.recovery) "Undoing…" else "Undo") }
                    }
                }
                errors[shownReceipt.recovery]?.let { HistoryError(it) }
            }
        }

        when (val current = load) {
            HistoryLoad.Loading -> Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(12.dp))
                Text("Loading history…", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            is HistoryLoad.Failed -> {
                HistoryError("Couldn't load history. ${current.message}")
                TextButton(onClick = { load = HistoryLoad.Loading; reload++ }, modifier = Modifier.heightIn(min = 48.dp)) {
                    ZIcon(ZIcons.Refresh, null, Modifier.size(18.dp)); Spacer(Modifier.width(8.dp)); Text("Retry")
                }
            }
            is HistoryLoad.Ready -> {
                val (recovery, points) = current.points.partition { it.recovery }
                if (points.isEmpty() && recovery.isEmpty()) {
                    Text("A checkpoint is saved after each finished turn.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 8.dp))
                }
                points.take(shown).forEach { point ->
                    key(point.id) {
                        CheckpointRow(point, chat, sharedTasks, reduced, restoring, errors[point.id], onRestore = { confirm = point })
                    }
                }
                if (points.size > shown) {
                    TextButton(onClick = { shown += 10 }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Show ${minOf(10, points.size - shown)} more") }
                }
                if (recovery.isNotEmpty()) {
                    val turn by animateFloatAsState(if (recoveryOpen) 90f else 0f, if (reduced) tween(0) else tween(180), label = "recovery-fold")
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp)
                            .semantics(mergeDescendants = true) { stateDescription = if (recoveryOpen) "Expanded" else "Collapsed" }
                            .clickable(role = Role.Button) { recoveryOpen = !recoveryOpen }.padding(horizontal = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp).rotate(turn), MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.width(12.dp))
                        Text("${recovery.size} recovery point${if (recovery.size == 1) "" else "s"}", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    if (recoveryOpen) recovery.forEach { point ->
                        key(point.id) {
                            CheckpointRow(point, chat, sharedTasks, reduced, restoring, errors[point.id], onRestore = { confirm = point })
                        }
                    }
                }
            }
            HistoryLoad.Unsupported -> Unit
        }
    }

    confirm?.let { point ->
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text(if (point.recovery) "Restore this recovery point?" else "Restore to this point?") },
            text = { Text(restoreBlurb(point)) },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Keep current") } },
            confirmButton = { TextButton(onClick = { confirm = null; rollback(point.id, point.title, undo = false) }) { Text("Restore") } },
        )
    }
}

/** One checkpoint: tap to inspect; Restore asks first. */
@Composable
private fun CheckpointRow(
    point: RunCheckpointView,
    chat: SessionRow,
    sharedTasks: Boolean,
    reduced: Boolean,
    restoring: String?,
    error: String?,
    onRestore: () -> Unit,
) {
    var expanded by remember(point.id) { mutableStateOf(false) }
    val blocked = restoreBlocked(point, chat, sharedTasks)
    val busy = restoring == point.id
    val motion = if (reduced) Modifier else Modifier.animateContentSize(tween(220))
    val wash by animateFloatAsState(if (expanded) 0.05f else 0f, if (reduced) tween(0) else tween(200), label = "checkpoint-wash")
    Column(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.medium).background(MaterialTheme.colorScheme.onSurface.copy(alpha = wash)).then(motion)) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 56.dp)
                .semantics(mergeDescendants = true) {
                    contentDescription = "${point.title}, saved ${ago(point.createdAtMs)}"
                    stateDescription = if (expanded) "Expanded" else "Collapsed"
                }
                .clickable(role = Role.Button) { expanded = !expanded }.padding(vertical = 10.dp, horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ZIcon(if (point.recovery) R.drawable.zi_reset else R.drawable.zi_clock_circle, null, Modifier.size(18.dp), MaterialTheme.colorScheme.onSurfaceVariant)
            Text(
                point.title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium,
                maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f).padding(horizontal = 12.dp),
            )
            Text(uniffi.zeron_core.relativeTimeLabel(atMs = point.createdAtMs, nowMs = System.currentTimeMillis()), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        AnimatedVisibility(expanded, enter = fadeIn(tween(if (reduced) 0 else 140)), exit = fadeOut(tween(if (reduced) 0 else 100))) {
            Column(Modifier.padding(start = 38.dp, end = 8.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    if (point.recovery) "Kept ${ago(point.createdAtMs)}, before a restore" else "Saved ${ago(point.createdAtMs)}",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    listOfNotNull(filesFact(point), when (point.providerStrategy) { "native" -> "Native resume"; "portable" -> "Portable"; else -> null }).joinToString(" • "),
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                strategyText(point.providerStrategy)?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                blocked?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.tertiary) }
                error?.let { HistoryError(it) }
                TextButton(
                    onClick = onRestore,
                    enabled = blocked == null && restoring == null,
                    modifier = Modifier.heightIn(min = 48.dp),
                ) {
                    if (busy) { CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp); Spacer(Modifier.width(8.dp)) }
                    Text(if (busy) "Restoring…" else if (error != null) "Try again" else if (point.recovery) "Restore this point" else "Restore to here")
                }
            }
        }
    }
}

@Composable
private fun HistoryError(text: String) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 8.dp).semantics { liveRegion = LiveRegionMode.Polite })
}
