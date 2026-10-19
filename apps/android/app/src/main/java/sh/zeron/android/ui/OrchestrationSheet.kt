package sh.zeron.android.ui

import android.database.ContentObserver
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.HapticFeedbackConstants
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.ChatIndicator
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.SessionRow

/** The same app-owned task states as desktop, presented as a touch sheet. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OrchestrationSheet(
    core: CoreClient,
    chat: SessionRow?,
    children: List<SessionRow>,
    onDismiss: () -> Unit,
    onOpen: (String) -> Unit,
) {
    val reduced = rememberOrchestrationReducedMotion()
    var finishedOpen by remember(chat?.id) { mutableStateOf(false) }
    val active = children.filter { !it.archived && it.task?.let { task -> !task.terminal || (!task.delivered && !task.cancelRequested) } == true }
        .sortedBy { it.task?.createdAtMs }
    val finished = children.filter { !it.archived && it.task?.let { task -> task.terminal && (task.delivered || task.cancelRequested) } == true }
        .sortedByDescending { it.task?.updatedAtMs }
    val sideChats = children.filter { it.task == null && !it.archived }
    val scope = rememberCoroutineScope()
    var forking by remember { mutableStateOf(false) }
    var forkError by remember { mutableStateOf<String?>(null) }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        LazyColumn(
            Modifier.fillMaxWidth().heightIn(max = 680.dp).navigationBarsPadding(),
            contentPadding = PaddingValues(start = 20.dp, end = 20.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            item { Text("Tasks and side chats", style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(bottom = 8.dp)) }
            (chat?.task?.parentChatId ?: chat?.fork?.sourceChatId ?: chat?.parentChatId)?.let { parent ->
                item {
                    val parentRow = core.sessionRow(chatId = parent)
                    TextButton(onClick = { onOpen(parent) }, enabled = parentRow != null, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        ZIcon(ZIcons.Back, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text(if (parentRow == null) "Parent chat unavailable" else if (chat?.task != null) "Back to delegating chat" else if (chat?.fork != null) "Back to source chat" else "Back to parent chat")
                    }
                }
            }
            if (active.isNotEmpty() || finished.isNotEmpty()) {
                item { LaneHeading("Tasks") }
                items(active, key = { "task-${it.id}" }) { task ->
                    TaskDetailRow(core, task, reduced, onOpen)
                }
                if (finished.isNotEmpty()) {
                    item {
                        TextButton(
                            onClick = { finishedOpen = !finishedOpen },
                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).semantics { stateDescription = if (finishedOpen) "Expanded" else "Collapsed" },
                        ) {
                            ZIcon(if (finishedOpen) ZIcons.ChevronUp else ZIcons.ChevronDown, null, Modifier.size(18.dp))
                            Spacer(Modifier.width(8.dp)); Text("${finished.size} finished")
                        }
                    }
                    if (finishedOpen) {
                        items(finished, key = { "task-${it.id}" }) { task -> TaskDetailRow(core, task, reduced, onOpen) }
                    }
                }
            }
            item { LaneHeading("Side chats") }
            items(sideChats, key = { "side-${it.id}" }) { row -> SideChatRow(core, row, chat?.deviceId, reduced, onOpen) }
            if (sideChats.isEmpty()) {
                item {
                    Text("Fork this conversation to explore another direction. Your current chat stays here.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            item {
                TextButton(
                    enabled = chat != null && chat.deviceOnline && !forking,
                    onClick = {
                        val source = chat ?: return@TextButton
                        forking = true; forkError = null
                        scope.launch {
                            try { val id = core.forkSession(sourceChatId = source.id); onOpen(id) }
                            catch (error: CancellationException) { throw error }
                            catch (error: Exception) { forkError = "Couldn't fork this chat. ${error.message.orEmpty()}" }
                            finally { forking = false }
                        }
                    },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
                ) {
                    if (forking) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else ZIcon(ZIcons.Branch, null, Modifier.size(20.dp))
                    Spacer(Modifier.width(8.dp)); Text(if (forking) "Creating side chat…" else "Fork this chat")
                }
                forkError?.let { ErrorNote(it) }
                if (chat?.deviceOnline == false) Text("Its device is offline", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (children.isEmpty()) {
                item { Text("Delegated work will appear here when an agent starts a task.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
            // The checkpoints each finished turn left, restorable in place.
            chat?.let { current -> item(key = "history-${current.id}") { CheckpointHistory(core, current, children, reduced) } }
        }
    }
}

@Composable
private fun LaneHeading(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 10.dp, bottom = 2.dp))
}

@Composable
private fun TaskDetailRow(core: CoreClient, row: SessionRow, reduced: Boolean, onOpen: (String) -> Unit) {
    val task = row.task ?: return
    var expanded by remember(row.id) { mutableStateOf(false) }
    var confirmStop by remember(row.id) { mutableStateOf(false) }
    var pending by remember(row.id) { mutableStateOf(false) }
    var requested by remember(row.id) { mutableStateOf(false) }
    var error by remember(row.id) { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val view = LocalView.current
    val wash = remember(row.id) { Animatable(0f) }
    var wasTerminal by remember(row.id) { mutableStateOf(task.terminal) }
    LaunchedEffect(task.cancelRequested, task.terminal) {
        if (task.cancelRequested || task.terminal) { requested = false; error = null }
        if (task.terminal) confirmStop = false
    }
    LaunchedEffect(task.terminal, reduced) {
        if (reduced) wash.snapTo(0f)
        if (!wasTerminal && task.terminal) {
            // A haptic is not motion: completion still taps under reduced motion.
            if (task.status == "completed") view.performHapticFeedback(if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.CONFIRM else HapticFeedbackConstants.KEYBOARD_TAP)
            if (!reduced) { wash.snapTo(0.12f); wash.animateTo(0f, tween(1100)) }
        }
        wasTerminal = task.terminal
    }
    val stopping = pending || requested || task.stopping
    val label = if (stopping) "Stopping…" else task.statusLabel
    val tone = when (task.status) {
        "failed" -> MaterialTheme.colorScheme.error
        "waiting" -> MaterialTheme.colorScheme.primary
        "cancelled", "interrupted" -> MaterialTheme.colorScheme.outline
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    val motion = if (reduced) Modifier else Modifier.animateContentSize(tween(220))
    // The settle washes in how it ended: success, error, or a neutral ink.
    val settleTone = when (task.status) {
        "completed" -> successColor()
        "failed" -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.onSurface
    }
    Column(Modifier.fillMaxWidth().clip(MaterialTheme.shapes.medium).background(settleTone.copy(alpha = wash.value)).then(motion)) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 56.dp)
                .semantics(mergeDescendants = true) { contentDescription = "${row.title}, $label"; stateDescription = if (expanded) "Expanded" else "Collapsed" }
                .clickable(role = Role.Button) { expanded = !expanded }.padding(vertical = 10.dp, horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TaskMark(task.status, stopping, reduced, tone)
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(row.title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(label, style = MaterialTheme.typography.bodySmall, color = tone)
            }
            Text(uniffi.zeron_core.relativeTimeLabel(atMs = task.updatedAtMs, nowMs = System.currentTimeMillis()), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.width(8.dp))
            ZIcon(if (expanded) ZIcons.ChevronUp else ZIcons.ChevronDown, null, Modifier.size(18.dp), MaterialTheme.colorScheme.onSurfaceVariant)
        }
        AnimatedVisibility(expanded, enter = fadeIn(tween(if (reduced) 0 else 140)), exit = fadeOut(tween(if (reduced) 0 else 100))) {
            Column(Modifier.padding(start = 36.dp, end = 8.dp, bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(listOfNotNull(task.roleLabel, task.isolationLabel, row.modelLabel ?: row.model).joinToString(" • "), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                task.summary?.takeIf { it.isNotBlank() }?.let {
                    Text(it, style = MaterialTheme.typography.bodyMedium, maxLines = 4, overflow = TextOverflow.Ellipsis)
                    // The record keeps a bounded summary; say when it was cut.
                    if (task.summaryTruncated) Text("Summary shortened. Open the task to read the full result.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (task.status == "waiting") Text("It's waiting on an answer. Open it to reply.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.primary)
                if (task.status == "waiting_for_children") Text("Waiting for its own tasks to finish.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (task.terminal) {
                    Text(if (task.delivered) "Sent to this chat" else if (task.cancelRequested) "Stopped from this chat" else "Result ready, not sent yet", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { onOpen(row.id) }, modifier = Modifier.heightIn(min = 48.dp)) { Text("Open task") }
                    if (!task.terminal) {
                        TextButton(onClick = { confirmStop = true }, enabled = row.deviceOnline && !stopping, modifier = Modifier.heightIn(min = 48.dp), colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)) { Text(if (stopping) "Stopping…" else if (error != null) "Try again" else "Stop task") }
                    }
                }
                error?.let { ErrorNote(it) }
                if (!row.deviceOnline && !task.terminal) Text("Its device is offline", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
    if (confirmStop) AlertDialog(
        onDismissRequest = { confirmStop = false }, title = { Text("Stop this task?") },
        text = { Text("This stops the task and the tasks it started. Forks and side chats keep running. A stopped task's result isn't sent here.") },
        dismissButton = { TextButton(onClick = { confirmStop = false }) { Text("Keep running") } },
        confirmButton = { TextButton(onClick = {
            confirmStop = false; pending = true; error = null
            scope.launch {
                try { core.cancelTask(parentChatId = task.parentChatId, taskId = task.id); requested = true }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (failure: Exception) { error = "Couldn't stop the task. ${failure.message.orEmpty()}" }
                finally { pending = false }
            }
        }, colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)) { Text("Stop") } },
    )
}

@Composable
private fun TaskMark(status: String, stopping: Boolean, reduced: Boolean, tone: Color) {
    if ((stopping || status in listOf("running", "waiting_for_children")) && !reduced) {
        CircularProgressIndicator(Modifier.size(18.dp), color = tone, strokeWidth = 2.dp)
    } else ZIcon(if (status == "completed") ZIcons.Check else if (status in listOf("cancelled", "interrupted")) ZIcons.Close else ZIcons.Chat, null, Modifier.size(18.dp), tone)
}

@Composable
private fun SideChatRow(core: CoreClient, row: SessionRow, parentDevice: String?, reduced: Boolean, onOpen: (String) -> Unit) {
    var confirming by remember(row.id) { mutableStateOf(false) }
    var pending by remember(row.id) { mutableStateOf(false) }
    // Kept for the sheet's life: the fork's own activity doesn't undo a merge.
    var sent by remember(row.id) { mutableStateOf(false) }
    var error by remember(row.id) { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val busy = row.hostIndicator == ChatIndicator.WORKING || row.hostIndicator == ChatIndicator.AWAITING_INPUT
    Column(Modifier.fillMaxWidth().then(if (reduced) Modifier else Modifier.animateContentSize(tween(220)))) {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).clickable(role = Role.Button) { onOpen(row.id) }.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
            ZIcon(if (row.fork != null) ZIcons.Branch else ZIcons.Chat, null, Modifier.size(20.dp), MaterialTheme.colorScheme.onSurfaceVariant)
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(row.title, style = MaterialTheme.typography.bodyLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
                val source = row.fork?.let { core.sessionRow(chatId = it.sourceChatId)?.title }
                Text(if (row.fork != null) "Forked from ${source ?: "another chat"}" else row.harnessLabel.orEmpty(), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            ZIcon(ZIcons.ChevronRight, null, Modifier.size(18.dp))
        }
        if (row.fork != null && row.deviceId == parentDevice) TextButton(onClick = { confirming = true }, enabled = !pending && !busy && row.deviceOnline && !sent, modifier = Modifier.padding(start = 28.dp).heightIn(min = 48.dp)) {
            Text(if (pending) "Sending result…" else if (sent) "Merged into this chat" else if (busy) "Finish its turn before merging" else "Merge into this chat")
        }
        error?.let { ErrorNote(it) }
    }
    if (confirming) AlertDialog(onDismissRequest = { confirming = false }, title = { Text("Merge this result?") }, text = { Text("Sends the side chat's latest completed response here as context.") },
        dismissButton = { TextButton(onClick = { confirming = false }) { Text("Not now") } },
        confirmButton = { TextButton(onClick = {
            confirming = false; pending = true; error = null
            scope.launch {
                try { core.mergeChat(sourceChatId = row.id); sent = true }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (failure: Exception) { error = "Couldn't merge. ${failure.message.orEmpty()}" }
                finally { pending = false }
            }
        }) { Text("Merge") } })
}

@Composable
private fun ErrorNote(text: String) { Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 8.dp).semantics { liveRegion = LiveRegionMode.Polite }) }

/** Respond to the OS motion setting without polling or adding another preference. */
@Composable
private fun rememberOrchestrationReducedMotion(): Boolean {
    val resolver = LocalContext.current.contentResolver
    fun read() = runCatching { Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }.getOrDefault(false)
    var reduced by remember(resolver) { mutableStateOf(read()) }
    DisposableEffect(resolver) {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) { override fun onChange(selfChange: Boolean) { reduced = read() } }
        resolver.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, observer)
        onDispose { resolver.unregisterContentObserver(observer) }
    }
    return reduced
}
