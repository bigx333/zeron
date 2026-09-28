package sh.zeron.android.ui

import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.CallSplit
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.DataUsage
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.MoreHoriz
import androidx.compose.material.icons.outlined.Send
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material.icons.outlined.Schedule
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import uniffi.zeron_core.BusyPolicy
import uniffi.zeron_core.ChatConfig
import uniffi.zeron_core.ComposerState
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.InputRequest
import uniffi.zeron_core.ModelInfo
import uniffi.zeron_core.QueueGate
import uniffi.zeron_core.QueueItem
import uniffi.zeron_core.SandboxLevel
import uniffi.zeron_core.SendRequest
import uniffi.zeron_core.SessionHandle
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.UserInputAnswer
import uniffi.zeron_core.fallbackModels
import uniffi.zeron_core.reasoningLabel

private enum class Delivery(val label: String) { Queue("Queue"), Steer("Steer"), Interrupt("Stop & send") }

/** The session composer: the shared surface plus this chat's context chips. */
@Composable
fun Composer(
    model: AppModel,
    client: CoreClient,
    handle: SessionHandle,
    c: ComposerState,
    row: SessionRow?,
    onSend: () -> Unit,
) {
    var text by rememberSaveable(c.chatId) { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var delivery by remember { mutableStateOf(Delivery.Queue) }
    val running = c.live.turnRunning
    val canSteer = c.host.capabilities.midTurnSteering == true

    fun send() {
        val body = text.trim()
        if (body.isEmpty()) return
        try {
            if (delivery == Delivery.Interrupt && running) handle.interrupt()
            handle.send(SendRequest(body, emptyList(), null, if (running && delivery == Delivery.Steer) BusyPolicy.STEER else BusyPolicy.QUEUE))
            text = ""
            error = null
            delivery = Delivery.Queue
            onSend()
        } catch (e: Exception) {
            error = "Couldn't send: ${e.message}"
        }
    }

    Column(Modifier.padding(horizontal = 12.dp).padding(bottom = 8.dp)) {
        error?.let {
            Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(start = 16.dp, bottom = 6.dp))
        }
        ComposerSurface(
            text = text,
            onText = { text = it },
            placeholder = "Message ${row?.harnessLabel ?: "the agent"}",
            action = when {
                running && text.isBlank() -> ComposerAction.Stop
                running -> ComposerAction.Queue
                else -> ComposerAction.Send
            },
            onAction = { if (running && text.isBlank()) runCatching { handle.interrupt() } else send() },
        ) {
            if (running) DeliveryChip(delivery, canSteer) { delivery = it }
            SessionChips(client, c, row)
        }
    }
}

@Composable
private fun DeliveryChip(current: Delivery, canSteer: Boolean, onChange: (Delivery) -> Unit) {
    var open by remember { mutableStateOf(false) }
    ContextChip(
        current.label,
        leading = { ZIcon(ZIcons.Queue, null, Modifier.size(16.dp)) },
        onClick = { open = true },
        tint = MaterialTheme.colorScheme.primary,
    ) {
        ChoiceMenu(
            open,
            { open = false },
            listOf(
                MenuSection(
                    "While the agent works",
                    listOfNotNull(
                        MenuChoice("Queue", current == Delivery.Queue, "Send when this turn ends") { onChange(Delivery.Queue) },
                        if (canSteer) MenuChoice("Steer", current == Delivery.Steer, "Add to the running turn") { onChange(Delivery.Steer) } else null,
                        MenuChoice("Stop & send", current == Delivery.Interrupt, "Stop the turn, then send") { onChange(Delivery.Interrupt) },
                    ),
                ),
            ),
        )
    }
}

@Composable
private fun SessionChips(client: CoreClient, c: ComposerState, row: SessionRow?) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val harness = row?.harness ?: "claude-code"
    var models by remember { mutableStateOf<List<ModelInfo>?>(null) }
    var menu by remember { mutableStateOf<String?>(null) }

    fun open(which: String) {
        menu = which
        if (models == null) scope.launch {
            models = runCatching { client.listModels(c.host.deviceId, harness) }.getOrNull() ?: fallbackModels(harness)
        }
    }

    fun setConfig(change: (ChatConfig) -> ChatConfig) {
        val current = client.sessionConfig(c.chatId) ?: ChatConfig(harness, row?.model, row?.reasoning, emptyMap(), SandboxLevel.WORKSPACE_WRITE)
        runCatching { client.setSessionConfig(c.chatId, change(current)) }
    }

    val modelLabel = row?.modelLabel ?: row?.harnessLabel
    if (modelLabel != null) {
        ContextChip(modelLabel, leading = { HarnessMark(harness, 14.dp) }, onClick = { open("model") }) {
            ChoiceMenu(menu == "model", { menu = null }, listOf(MenuSection(row?.harnessLabel, models.orEmpty().map { m ->
                MenuChoice(m.label, m.id == row?.model, m.description) { setConfig { it.copy(model = m.id) } }
            })))
        }
    }
    row?.reasoning?.takeIf { it.isNotEmpty() }?.let { level ->
        ContextChip(reasoningLabel(level), leading = { ZIcon(ZIcons.Effort, null, Modifier.size(16.dp)) }, onClick = { open("effort") }) {
            val levels = models?.let { list -> (list.firstOrNull { it.id == row.model } ?: list.firstOrNull())?.reasoningLevels }.orEmpty()
            ChoiceMenu(menu == "effort", { menu = null }, listOf(MenuSection("Reasoning effort", levels.map { l ->
                MenuChoice(reasoningLabel(l), l == level) { setConfig { it.copy(reasoning = l) } }
            })))
        }
    }
    val pr = row?.pullRequest
    if (pr != null) {
        ContextChip("#${pr.number}", leading = { ZIcon(ZIcons.PullRequest, null, Modifier.size(16.dp)) }, onClick = {
            CustomTabsIntent.Builder().build().launchUrl(context, Uri.parse(pr.url))
        })
    } else {
        row?.branch?.takeIf { it.isNotEmpty() }?.let {
            ContextChip(it, leading = { ZIcon(ZIcons.Branch, null, Modifier.size(16.dp)) }, onClick = {})
        }
    }
    val tokens = c.contextUsage?.tokens
    val window = c.contextUsage?.window
    if (tokens != null && window != null && window > 0u) {
        val fraction = tokens.toDouble() / window.toDouble()
        if (fraction >= 0.5) {
            ContextChip(
                "${(fraction * 100).toInt()}% context",
                leading = { ZIcon(if (fraction >= 0.85) ZIcons.Warning else ZIcons.Context, null, Modifier.size(16.dp)) },
                onClick = {},
                tint = if (fraction >= 0.85) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
fun StatusBanner(text: String, action: Pair<String, () -> Unit>?) {
    Surface(
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
    ) {
        Row(Modifier.padding(start = 16.dp, end = if (action != null) 4.dp else 16.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(vertical = 10.dp).weight(1f, fill = false))
            action?.let { (label, run) -> TextButton(onClick = run) { Text(label) } }
        }
    }
}

/** The agent asked something: answer with its options instead of typing. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class, ExperimentalLayoutApi::class)
@Composable
fun QuestionPanel(input: InputRequest, onSubmit: (List<UserInputAnswer>) -> Unit) {
    val picked = remember(input.requestId) { input.questions.associate { it.id to mutableStateListOf<String>() } }
    Surface(
        shape = RoundedCornerShape(28.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 4.dp),
    ) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            for (q in input.questions) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (q.header.isNotEmpty()) Text(q.header, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                    Text(q.question, style = MaterialTheme.typography.bodyLarge)
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        val selection = picked.getValue(q.id)
                        for (option in q.options) {
                            ToggleButton(
                                checked = option in selection,
                                onCheckedChange = { on ->
                                    if (!q.multiSelect) selection.clear()
                                    if (on) selection.add(option) else selection.remove(option)
                                },
                            ) { Text(option) }
                        }
                    }
                }
            }
            val ready = input.questions.all { picked.getValue(it.id).isNotEmpty() }
            Button(
                onClick = { onSubmit(input.questions.map { UserInputAnswer(it.id, picked.getValue(it.id).toList()) }) },
                enabled = ready,
                shapes = ButtonDefaults.shapes(),
                modifier = Modifier.align(Alignment.End),
            ) {
                ZIcon(ZIcons.Send, null, Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text("Answer")
            }
        }
    }
}

/** Messages queued behind the live turn (shared across devices). */
@Composable
fun QueuePanel(queue: List<QueueItem>, handle: SessionHandle) {
    val scope = rememberCoroutineScope()
    Column(Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 4.dp)) {
        queue.forEachIndexed { i, item ->
            Surface(
                shape = segmentShape(i, queue.size, outer = 20.dp, inner = 4.dp),
                color = MaterialTheme.colorScheme.surfaceContainer,
                modifier = Modifier.fillMaxWidth().padding(bottom = 2.dp),
            ) {
                Row(Modifier.padding(start = 16.dp, end = 4.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("Queued", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary)
                        Text(
                            item.visibleText.ifEmpty { "Attachment" },
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        gate(item)?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                    var menu by remember { mutableStateOf(false) }
                    Box {
                        IconButton(onClick = { menu = true }) { ZIcon(ZIcons.More, "Queued message actions", Modifier.size(20.dp)) }
                        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                            DropdownMenuItem(
                                text = { Text("Send now") },
                                leadingIcon = { ZIcon(ZIcons.Send, null, Modifier.size(20.dp)) },
                                onClick = { menu = false; scope.launch { runCatching { handle.deliverQueuedNow(item.id) } } },
                            )
                            if (i > 0) DropdownMenuItem(
                                text = { Text("Move up") },
                                leadingIcon = { ZIcon(ZIcons.ChevronUp, null, Modifier.size(20.dp)) },
                                onClick = { menu = false; runCatching { handle.moveQueuedBy(item.id, -1) } },
                            )
                            if (i < queue.size - 1) DropdownMenuItem(
                                text = { Text("Move down") },
                                leadingIcon = { ZIcon(ZIcons.ChevronDown, null, Modifier.size(20.dp)) },
                                onClick = { menu = false; runCatching { handle.moveQueuedBy(item.id, 1) } },
                            )
                            DropdownMenuItem(
                                text = { Text("Remove") },
                                leadingIcon = { ZIcon(ZIcons.Delete, null, Modifier.size(20.dp)) },
                                onClick = { menu = false; scope.launch { runCatching { handle.removeQueued(item.id) } } },
                            )
                        }
                    }
                }
            }
        }
    }
}

private fun gate(item: QueueItem): String? = when (val g = item.gate) {
    is QueueGate.Editing -> if (g.mine) "Editing" else "Being edited"
    is QueueGate.ReviewRequired -> "Needs review"
    null -> if (item.actionPending) "Updating" else null
}
