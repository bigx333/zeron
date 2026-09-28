package sh.zeron.android.ui

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.CallSplit
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.outlined.Archive
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.DriveFileRenameOutline
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SmallExtendedFloatingActionButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.CircleShape
import sh.zeron.android.design.HarnessMark
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import uniffi.zeron_core.ChatIndicator
import uniffi.zeron_core.ConnectivityState
import uniffi.zeron_core.PullRequestState
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.WorkspaceSnapshot

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SessionsScreen(model: AppModel, onOpen: (String) -> Unit, onNew: () -> Unit, onSearch: () -> Unit) {
    val workspace by model.workspace.collectAsState()
    val connectivity by model.connectivity.collectAsState()
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val list = rememberLazyListState()
    val expanded by remember { derivedStateOf { list.firstVisibleItemIndex == 0 } }
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    val pull = rememberPullToRefreshState()

    val ws = workspace
    val live = remember(ws) { ws?.let { liveCounts(it) } ?: (0 to 0) }
    val subtitle = when {
        connectivity?.state == ConnectivityState.OFFLINE -> "Offline"
        connectivity?.state == ConnectivityState.RECONNECTING -> "Reconnecting…"
        model.isDemo -> "Demo workspace"
        live.first > 0 || live.second > 0 -> listOfNotNull(
            if (live.first > 0) "${live.first} working" else null,
            if (live.second > 0) "${live.second} need you" else null,
        ).joinToString(" · ")
        else -> null
    }

    val archive: (SessionRow) -> Unit = { row ->
        model.archive(row.id)
        scope.launch {
            if (snackbar.showSnackbar("Archived", actionLabel = "Undo", withDismissAction = true) == SnackbarResult.ActionPerformed) {
                model.unarchive(row.id)
            }
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            LargeFlexibleTopAppBar(
                title = { Text("Sessions") },
                subtitle = subtitle?.let { { Text(it) } },
                actions = {
                    if (connectivity?.state == ConnectivityState.OFFLINE) {
                        Icon(Icons.Outlined.CloudOff, "Offline", Modifier.padding(end = 4.dp), tint = MaterialTheme.colorScheme.error)
                    }
                    IconButton(onClick = onSearch) { Icon(Icons.Outlined.Search, "Search") }
                },
                scrollBehavior = scroll,
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
            )
        },
        floatingActionButton = {
            SmallExtendedFloatingActionButton(
                onClick = onNew,
                expanded = expanded,
                icon = { Icon(Icons.Filled.Edit, null) },
                text = { Text("New session") },
            )
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                refreshing = true
                scope.launch {
                    model.refresh()
                    refreshing = false
                }
            },
            state = pull,
            modifier = Modifier.padding(top = padding.calculateTopPadding()).fillMaxSize(),
            indicator = {
                PullToRefreshDefaults.LoadingIndicator(state = pull, isRefreshing = refreshing, modifier = Modifier.align(Alignment.TopCenter))
            },
        ) {
            LazyColumn(
                state = list,
                contentPadding = PaddingValues(bottom = 120.dp),
                modifier = Modifier.fillMaxSize(),
            ) {
                if (ws == null) return@LazyColumn
                val front = ws.front
                if (front.pinned.isNotEmpty()) group("pinned", "Pinned", front.pinned, model, onOpen, archive)
                for (section in front.sections) {
                    group(section.id, section.name, if (section.collapsed) emptyList() else section.sessions, model, onOpen, archive) {
                        IconButton(onClick = { model.setSectionCollapsed(section.id, !section.collapsed) }) {
                            Icon(if (section.collapsed) Icons.Outlined.ExpandMore else Icons.Outlined.ExpandLess, if (section.collapsed) "Expand" else "Collapse")
                        }
                    }
                }
                if (front.recent.isNotEmpty()) group("recent", "Recent", front.recent, model, onOpen, archive)
                if (front.pinned.isEmpty() && front.sections.isEmpty() && front.recent.isEmpty()) {
                    item("empty") { EmptyState() }
                }
            }
        }
    }
}

private fun liveCounts(ws: WorkspaceSnapshot): Pair<Int, Int> {
    val seen = HashSet<String>()
    var working = 0
    var awaiting = 0
    val all = ws.front.pinned + ws.front.sections.flatMap { it.sessions } + ws.front.recent
    for (r in all) {
        if (!seen.add(r.id)) continue
        if (r.indicator == ChatIndicator.WORKING) working++
        if (r.indicator == ChatIndicator.AWAITING_INPUT) awaiting++
    }
    return working to awaiting
}

private fun LazyListScope.group(
    id: String,
    title: String,
    rows: List<SessionRow>,
    model: AppModel,
    onOpen: (String) -> Unit,
    archive: (SessionRow) -> Unit,
    trailing: (@Composable () -> Unit)? = null,
) {
    item("h-$id") { SectionHeader(if (rows.isEmpty()) title else "$title  ${rows.size}", Modifier.animateItem(), trailing) }
    itemsIndexed(rows, key = { _, r -> "$id/${r.id}" }) { i, row ->
        Box(Modifier.animateItem().padding(horizontal = 12.dp).padding(bottom = if (i < rows.size - 1) 2.dp else 0.dp)) {
            SwipeableSessionRow(row, segmentShape(i, rows.size), model, onOpen, archive)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SwipeableSessionRow(
    row: SessionRow,
    shape: androidx.compose.ui.graphics.Shape,
    model: AppModel,
    onOpen: (String) -> Unit,
    archive: (SessionRow) -> Unit,
) {
    val state = rememberSwipeToDismissBoxState()
    SwipeToDismissBox(
        state = state,
        enableDismissFromStartToEnd = false,
        onDismiss = { if (it == SwipeToDismissBoxValue.EndToStart) archive(row) },
        backgroundContent = {
            Surface(Modifier.fillMaxSize(), shape = shape, color = MaterialTheme.colorScheme.tertiaryContainer) {
                Box(Modifier.padding(end = 24.dp), contentAlignment = Alignment.CenterEnd) {
                    Icon(Icons.Outlined.Archive, "Archive", tint = MaterialTheme.colorScheme.onTertiaryContainer)
                }
            }
        },
    ) {
        SessionItem(row, shape, model, onOpen, archive)
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun SessionItem(
    row: SessionRow,
    shape: androidx.compose.ui.graphics.Shape,
    model: AppModel,
    onOpen: (String) -> Unit,
    archive: (SessionRow) -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    Surface(
        shape = shape,
        color = MaterialTheme.colorScheme.surfaceContainerLowest,
        modifier = Modifier
            .fillMaxWidth()
            .clip(shape)
            .combinedClickable(
                onClick = { onOpen(row.id) },
                onLongClick = {
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    menu = true
                },
            ),
    ) {
        Row(
            Modifier.padding(start = 12.dp, end = 16.dp, top = 12.dp, bottom = 12.dp).animateContentSize(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(MaterialTheme.colorScheme.surfaceContainer),
                contentAlignment = Alignment.Center,
            ) { HarnessMark(row.harness, 20.dp, tint = MaterialTheme.colorScheme.onSurface) }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    row.title,
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.Medium),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Spacer(Modifier.height(3.dp))
                Subline(row)
            }
            Spacer(Modifier.width(10.dp))
            Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                StatusLabel(row)
                row.pullRequest?.let { PrBadge(it.number, it.state) }
            }
        }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            DropdownMenuItem(
                text = { Text(if (row.pinned) "Unpin" else "Pin") },
                leadingIcon = { Icon(Icons.Outlined.PushPin, null) },
                onClick = { model.setPinned(row.id, !row.pinned); menu = false },
            )
            DropdownMenuItem(
                text = { Text("Rename") },
                leadingIcon = { Icon(Icons.Outlined.DriveFileRenameOutline, null) },
                onClick = { renaming = true; menu = false },
            )
            DropdownMenuItem(
                text = { Text("Archive") },
                leadingIcon = { Icon(Icons.Outlined.Archive, null) },
                onClick = { archive(row); menu = false },
            )
        }
    }
    if (renaming) RenameDialog(row.title, onDismiss = { renaming = false }) { model.rename(row.id, it) }
}

fun subtitle(row: SessionRow): String = listOfNotNull(
    row.project?.name ?: row.deviceName ?: "No project",
    row.branch?.takeIf { it.isNotEmpty() && row.pullRequest == null },
).joinToString(" · ")

/** Project monogram + name, then the branch — the desktop sidebar subline. */
@Composable
private fun Subline(row: SessionRow) {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Row(verticalAlignment = Alignment.CenterVertically) {
        val project = row.project
        if (project != null) {
            ProjectTile(project.name, project.colorIndex.toInt(), 16.dp)
            Spacer(Modifier.width(6.dp))
        }
        Text(
            project?.name ?: row.deviceName ?: "No project",
            style = MaterialTheme.typography.bodyMedium,
            color = muted,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f, fill = false),
        )
        row.branch?.takeIf { it.isNotEmpty() }?.let { branch ->
            Spacer(Modifier.width(10.dp))
            Icon(Icons.AutoMirrored.Outlined.CallSplit, null, Modifier.size(13.dp), tint = muted.copy(alpha = 0.6f))
            Spacer(Modifier.width(3.dp))
            Text(branch, style = MaterialTheme.typography.bodyMedium, color = muted.copy(alpha = 0.6f), maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
fun PrBadge(number: ULong, state: PullRequestState) {
    val tone = when (state) {
        PullRequestState.OPEN -> successColor()
        PullRequestState.MERGED -> MaterialTheme.colorScheme.primary
        PullRequestState.CLOSED -> MaterialTheme.colorScheme.error
    }
    Surface(shape = RoundedCornerShape(8.dp), color = tone.copy(alpha = 0.12f), contentColor = tone) {
        Row(Modifier.padding(horizontal = 6.dp, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Outlined.CallSplit, null, Modifier.size(12.dp))
            Spacer(Modifier.width(3.dp))
            Text("$number", style = MaterialTheme.typography.labelMedium)
        }
    }
}

@Composable
fun RenameDialog(current: String, onDismiss: () -> Unit, onRename: (String) -> Unit) {
    var text by remember { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Rename session") },
        text = { OutlinedTextField(text, { text = it }, singleLine = true, shape = RoundedCornerShape(16.dp)) },
        confirmButton = {
            TextButton(onClick = { onRename(text.trim()); onDismiss() }, enabled = text.isNotBlank()) { Text("Rename") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun EmptyState() {
    Column(Modifier.fillMaxWidth().padding(48.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Text("No sessions yet", style = MaterialTheme.typography.titleLarge)
        Spacer(Modifier.height(8.dp))
        Text(
            "Start one on any of your devices.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
