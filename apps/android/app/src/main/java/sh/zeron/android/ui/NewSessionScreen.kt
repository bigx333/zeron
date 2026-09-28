package sh.zeron.android.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.CallSplit
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.NewSessionDraft
import uniffi.zeron_core.ModelInfo
import uniffi.zeron_core.fallbackHarnesses
import uniffi.zeron_core.fallbackModels
import uniffi.zeron_core.reasoningLabel

private data class ModelChoice(val harness: String, val harnessLabel: String, val model: ModelInfo)

private data class Place(val projectId: String?, val hostId: String, val name: String, val detail: String, val colorIndex: Int, val git: Boolean, val online: Boolean)

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun NewSessionScreen(model: AppModel, onClose: () -> Unit, onCreated: (String) -> Unit) {
    val workspace by model.workspace.collectAsState()
    val client by model.client.collectAsState()
    var draft by remember { mutableStateOf(model.lastDraft) }
    var text by rememberSaveable { mutableStateOf("") }
    var picking by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    val places = remember(workspace) {
        val ws = workspace ?: return@remember emptyList()
        ws.projects.map { Place(it.id, it.deviceId, it.name, it.deviceName ?: "", it.colorIndex.toInt(), it.gitDetected, it.deviceOnline) } +
            ws.devices.filter { it.isExecutionHost }.map { Place(null, it.id, "Home folder", it.name, uniffi.zeron_core.projectColorIndex("home").toInt(), false, it.online) }
    }.sortedByDescending { it.online }
    val selected = places.firstOrNull { p -> if (draft.projectId != null) p.projectId == draft.projectId else p.projectId == null && p.hostId == draft.hostId }
        ?: places.firstOrNull()
    LaunchedEffect(selected) {
        if (selected != null && (draft.projectId != selected.projectId || draft.hostId != selected.hostId)) {
            draft = draft.copy(projectId = selected.projectId, hostId = selected.hostId)
        }
    }

    // Models offered by the selected host (live catalog, static fallback).
    var choices by remember { mutableStateOf<List<ModelChoice>>(emptyList()) }
    LaunchedEffect(selected?.hostId, client) {
        val c = client ?: return@LaunchedEffect
        val host = selected?.hostId ?: return@LaunchedEffect
        val harnesses = runCatching { c.listHarnesses(host) }.getOrNull() ?: fallbackHarnesses()
        val out = ArrayList<ModelChoice>()
        for (h in harnesses.filter { it.offered }) {
            val models = runCatching { c.listModels(host, h.id) }.getOrNull() ?: fallbackModels(h.id)
            models.forEach { out.add(ModelChoice(h.id, h.label, it)) }
        }
        choices = out
        if (out.none { it.harness == draft.harness && it.model.id == draft.model }) {
            out.firstOrNull { it.harness == draft.harness }?.let { draft = draft.copy(model = it.model.id, effort = it.model.defaultReasoning) }
                ?: out.firstOrNull()?.let { draft = draft.copy(harness = it.harness, model = it.model.id, effort = it.model.defaultReasoning) }
        }
    }
    val choice = choices.firstOrNull { it.harness == draft.harness && it.model.id == draft.model }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                title = { Text("New session") },
                navigationIcon = { IconButton(onClick = onClose) { Icon(Icons.Filled.Close, "Close") } },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
            )
        },
    ) { padding ->
        Column(
            Modifier
                .padding(top = padding.calculateTopPadding())
                .fillMaxSize()
                .imePadding()
                .navigationBarsPadding(),
        ) {
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                TextField(
                    text,
                    { text = it },
                    placeholder = { Text("What should the agent do?") },
                    textStyle = MaterialTheme.typography.bodyLarge,
                    shape = RoundedCornerShape(28.dp),
                    colors = TextFieldDefaults.colors(
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
                        focusedIndicatorColor = Color.Transparent,
                        unfocusedIndicatorColor = Color.Transparent,
                    ),
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp).heightIn(min = 160.dp),
                )

                SectionHeader("Where")
                Row(
                    Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    for (p in places) {
                        val on = p == selected
                        Surface(
                            shape = RoundedCornerShape(if (on) 28.dp else 20.dp),
                            color = if (on) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.surfaceContainerLowest,
                            border = if (on) BorderStroke(2.dp, MaterialTheme.colorScheme.primary) else null,
                            modifier = Modifier.width(156.dp).clip(RoundedCornerShape(if (on) 28.dp else 20.dp)).clickable {
                                draft = draft.copy(projectId = p.projectId, hostId = p.hostId, worktree = draft.worktree && p.git)
                            },
                        ) {
                            Column(Modifier.padding(14.dp)) {
                                ProjectTile(if (p.projectId != null) p.name else null, p.colorIndex, 36.dp)
                                Spacer(Modifier.height(10.dp))
                                Text(p.name, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(
                                    if (p.online) p.detail else "${p.detail} · offline",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                    }
                }

                SectionHeader("Agent")
                val rows = 1 + (if (selected?.git == true) 1 else 0)
                SegmentedGroup(rows) { i, shape ->
                    when (i) {
                        0 -> ListItem(
                            headlineContent = { Text(choice?.model?.label ?: "Choose a model") },
                            supportingContent = { Text(choice?.harnessLabel ?: "") },
                            leadingContent = { Icon(Icons.Outlined.AutoAwesome, null) },
                            modifier = Modifier.clip(shape).clickable { picking = true },
                            colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest),
                        )
                        else -> ListItem(
                            headlineContent = { Text("New worktree") },
                            supportingContent = { Text("Run isolated from the checkout on the host") },
                            leadingContent = { Icon(Icons.AutoMirrored.Outlined.CallSplit, null) },
                            trailingContent = { Switch(draft.worktree, { draft = draft.copy(worktree = it) }) },
                            modifier = Modifier.clip(shape),
                            colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest),
                        )
                    }
                }
                val levels = choice?.model?.reasoningLevels.orEmpty()
                if (levels.isNotEmpty()) {
                    SectionHeader("Effort")
                    Row(
                        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp),
                        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
                    ) {
                        levels.forEachIndexed { i, level ->
                            ToggleButton(
                                checked = (draft.effort ?: choice?.model?.defaultReasoning) == level,
                                onCheckedChange = { draft = draft.copy(effort = level) },
                                shapes = when (i) {
                                    0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                                    levels.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                                    else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                                },
                            ) { Text(reasoningLabel(level)) }
                        }
                    }
                }
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(20.dp)) }
            }
            Button(
                onClick = {
                    model.lastDraft = draft
                    val id = model.createSession(draft, text.trim())
                    if (id != null) onCreated(id) else error = "Couldn't start the session."
                },
                enabled = text.isNotBlank() && selected != null,
                shapes = ButtonDefaults.shapes(),
                modifier = Modifier.fillMaxWidth().padding(16.dp).heightIn(min = ButtonDefaults.MediumContainerHeight),
                contentPadding = ButtonDefaults.contentPaddingFor(ButtonDefaults.MediumContainerHeight),
            ) {
                Icon(Icons.Filled.ArrowUpward, null, Modifier.size(ButtonDefaults.iconSizeFor(ButtonDefaults.MediumContainerHeight)))
                Spacer(Modifier.width(ButtonDefaults.iconSpacingFor(ButtonDefaults.MediumContainerHeight)))
                Text("Start session", style = ButtonDefaults.textStyleFor(ButtonDefaults.MediumContainerHeight))
            }
        }
    }

    if (picking) {
        ModalBottomSheet(onDismissRequest = { picking = false }) {
            Column(Modifier.verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
                for ((harness, group) in choices.groupBy { it.harnessLabel }) {
                    SectionHeader(harness)
                    for (m in group) {
                        ListItem(
                            headlineContent = { Text(m.model.label) },
                            supportingContent = m.model.description?.let { { Text(it, maxLines = 2) } },
                            trailingContent = { if (m == choice) Icon(Icons.Outlined.Check, null, tint = MaterialTheme.colorScheme.primary) },
                            modifier = Modifier.clickable {
                                draft = draft.copy(harness = m.harness, model = m.model.id, effort = m.model.defaultReasoning)
                                picking = false
                            },
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        )
                    }
                }
            }
        }
    }
}
