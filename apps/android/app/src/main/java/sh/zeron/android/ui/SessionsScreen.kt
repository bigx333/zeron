package sh.zeron.android.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.ui.focus.focusRequester
import sh.zeron.android.core.projectGroups
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.core.Shelf
import sh.zeron.android.core.SidebarMode
import sh.zeron.android.core.activeRows
import sh.zeron.android.core.frontFor
import sh.zeron.android.voice.rememberReducedMotion
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.ui.draw.scale
import androidx.compose.material3.toShape
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ProjectColors
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.ChatIndicator
import uniffi.zeron_core.ConnectivityState
import uniffi.zeron_core.PullRequestState
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.WorkspaceSnapshot

private enum class Filter(val label: String) { All("All"), NeedsYou("Needs you"), Working("Working"), Pinned("Pinned") }

/** "2 working · 1 needs you" — the live summary beside New session. */
fun liveSummary(ws: WorkspaceSnapshot): String? {
    val (working, awaiting) = liveCounts(ws)
    val parts = listOfNotNull(
        if (working > 0) "$working working" else null,
        if (awaiting > 0) "$awaiting need${if (awaiting == 1) "s" else ""} you" else null,
    )
    return parts.joinToString(" · ").ifEmpty { null }
}

private fun liveCounts(ws: WorkspaceSnapshot): Pair<Int, Int> {
    val rows = ws.front.activeRows()
    return rows.count { it.indicator == ChatIndicator.WORKING } to rows.count { it.indicator == ChatIndicator.AWAITING_INPUT }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SessionsScreen(model: AppModel, onOpen: (String) -> Unit, onArchived: () -> Unit, selected: String? = null) {
    val workspace by model.workspace.collectAsState()
    val connectivity by model.connectivity.collectAsState()
    val groupByProject by model.groupByProject.collectAsState()
    val collapsedGroups by model.collapsed.collectAsState()
    val mode by model.sidebarMode.collectAsState()
    val openShelves by model.openShelves.collectAsState()
    val reducedMotion = rememberReducedMotion()
    var filter by rememberSaveable { mutableStateOf(Filter.All) }
    val scope = rememberCoroutineScope()
    var options by remember { mutableStateOf(false) }
    var newSection by remember { mutableStateOf<String?>(null) }
    var renameSection by remember { mutableStateOf<uniffi.zeron_core.SectionView?>(null) }
    var reorder by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val pull = rememberPullToRefreshState()
    val ws = workspace
    val counts = remember(ws, mode) {
        ws?.frontFor(mode)?.activeRows()?.let { rows ->
            rows.count { it.indicator == ChatIndicator.WORKING } to rows.count { it.indicator == ChatIndicator.AWAITING_INPUT }
        } ?: (0 to 0)
    }

    val subtitle = when {
        connectivity?.state == ConnectivityState.OFFLINE -> "Offline"
        connectivity?.state == ConnectivityState.RECONNECTING -> "Reconnecting…"
        model.isDemo -> "Demo workspace"
        else -> model.accountDetail.takeIf { it.isNotEmpty() }
    }

    val archive: (SessionRow) -> Boolean = { row -> model.archive(row.id) }
    // "New section…" from a row's Move menu files that row into it.
    val actions = RowActions(
        model = model,
        onOpen = onOpen,
        archive = archive,
        sections = ws?.front?.sections.orEmpty().map { it.id to it.name },
        newSection = { chatId -> newSection = chatId ?: "" },
        selected = selected,
        mode = mode,
        reducedMotion = reducedMotion,
    )

    val list = androidx.compose.foundation.lazy.rememberLazyListState()
    val density = androidx.compose.ui.platform.LocalDensity.current
    Box(Modifier.fillMaxSize()) {
        sh.zeron.android.design.WallpaperHero(
            model.wallpaper,
            scrollFade = {
                if (list.firstVisibleItemIndex > 0) 0f
                else 1f - list.firstVisibleItemScrollOffset / (with(density) { 400.dp.toPx() })
            },
        )
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
            modifier = Modifier.fillMaxSize(),
            indicator = {
                PullToRefreshDefaults.LoadingIndicator(
                    state = pull,
                    isRefreshing = refreshing,
                    modifier = Modifier.align(Alignment.TopCenter).padding(WindowInsets.statusBars.asPaddingValues()),
                )
            },
        ) {
            LazyColumn(
                state = list,
                contentPadding = PaddingValues(top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 8.dp, bottom = 200.dp),
                modifier = Modifier.fillMaxSize(),
            ) {
                item("header") {
                    // The title says which layout is up (it cross-fades on the toggle).
                    ScreenHeader(if (mode.isT3) "Inbox" else "Sessions", subtitle) {
                        if (connectivity?.state == ConnectivityState.OFFLINE) {
                            TonalCircleButton(
                                ZIcons.Offline, "Offline", onClick = { scope.launch { model.refresh() } },
                                container = MaterialTheme.colorScheme.errorContainer, content = MaterialTheme.colorScheme.onErrorContainer,
                            )
                        }
                        Box {
                            TonalCircleButton(ZIcons.More, "Session options", onClick = { options = true })
                            ActionMenu(
                                options,
                                { options = false },
                                listOf(
                                    MenuAction("T3 Mode", ZIcons.Inbox, checked = mode.isT3) {
                                        model.setSidebarMode(if (mode.isT3) SidebarMode.Default else SidebarMode.T3)
                                    },
                                    MenuAction("New section…", ZIcons.Folder) { newSection = "" },
                                    MenuAction("Collapse all", ZIcons.Collapse) {
                                        val front = ws?.frontFor(mode) ?: return@MenuAction
                                        for (section in front.sections) if (!section.collapsed) model.setSectionCollapsed(section.id, true)
                                        val local = listOf("pinned", "recent") +
                                            if (groupByProject) projectGroups(front.recent, ws.projects).map { it.id } else emptyList()
                                        local.forEach { model.setGroupCollapsed(it, true) }
                                        if (mode.isT3) Shelf.entries.forEach { model.setShelfOpen(it, false) }
                                    },
                                    MenuAction("Group by project", ZIcons.Project, checked = groupByProject) { model.setGroupByProject(!groupByProject) },
                                    MenuAction(mode.settled, ZIcons.Archive, onClick = onArchived),
                                ),
                            )
                        }
                    }
                }
                item("filters") {
                    Row(
                        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        for (f in Filter.entries) {
                            Pill(
                                f.label,
                                filter == f,
                                onClick = { filter = f },
                                count = when (f) {
                                    Filter.NeedsYou -> counts.second
                                    Filter.Working -> counts.first
                                    else -> null
                                },
                            )
                        }
                    }
                }
                if (ws == null) return@LazyColumn
                // T3: pins, sections and the attention-first inbox hold only what
                // needs a look; Working and Snoozed sit on shelves below.
                val front = ws.frontFor(mode)
                val active = front.activeRows()
                when (filter) {
                    Filter.All -> {
                        if (front.pinned.isNotEmpty()) {
                            group(
                                "pinned", "Pinned", front.pinned, actions,
                                collapsed = "pinned" in collapsedGroups,
                                onToggle = { model.setGroupCollapsed("pinned", "pinned" !in collapsedGroups) },
                                menu = listOf(MenuAction("Reorder…", ZIcons.Sort) { reorder = true }),
                            )
                        }
                        for (section in front.sections) {
                            group(
                                section.id, section.name, section.sessions, actions,
                                collapsed = section.collapsed,
                                onToggle = { model.setSectionCollapsed(section.id, !section.collapsed) },
                                menu = listOf(
                                    MenuAction("Rename section…", ZIcons.Rename) { renameSection = section },
                                    MenuAction("Delete section", ZIcons.Delete, destructive = true) { model.deleteSection(section.id) },
                                ),
                            )
                        }
                        if (groupByProject) {
                            // Checkouts of one repository together; project-less sessions per device.
                            for (g in projectGroups(front.recent, ws.projects)) {
                                group(
                                    g.id, g.name, g.sessions, actions,
                                    collapsed = g.id in collapsedGroups,
                                    onToggle = { model.setGroupCollapsed(g.id, g.id !in collapsedGroups) },
                                )
                            }
                        } else if (front.recent.isNotEmpty()) {
                            group(
                                "recent", if (mode.isT3) "Inbox" else "Recent", front.recent, actions,
                                collapsed = "recent" in collapsedGroups,
                                onToggle = { model.setGroupCollapsed("recent", "recent" !in collapsedGroups) },
                            )
                        }
                    }
                    Filter.NeedsYou -> group("f", null, active.filter { it.indicator == ChatIndicator.AWAITING_INPUT }, actions)
                    Filter.Working -> group("f", null, active.filter { it.indicator == ChatIndicator.WORKING }, actions)
                    Filter.Pinned -> group("f", null, front.pinned, actions)
                }
                val empty = when (filter) {
                    Filter.All -> front.pinned.isEmpty() && front.sections.isEmpty() && front.recent.isEmpty()
                    Filter.NeedsYou -> counts.second == 0
                    Filter.Working -> counts.first == 0
                    Filter.Pinned -> front.pinned.isEmpty()
                }
                if (empty) item("empty") { EmptyState(filter, mode, itemMotion(reducedMotion)) }
                if (mode.isT3 && filter == Filter.All) {
                    shelf(Shelf.Working, front.working, front.working.size, openShelves, actions) { model.setShelfOpen(Shelf.Working, it) }
                    shelf(Shelf.Snoozed, front.snoozed, front.snoozed.size, openShelves, actions) { model.setShelfOpen(Shelf.Snoozed, it) }
                }
            }
        }
        StatusBarScrim(scrolled = list.firstVisibleItemIndex > 0 || list.firstVisibleItemScrollOffset > 0)
    }

    newSection?.let { chatId ->
        TextPromptDialog("New section", "", "Create", ZIcons.Folder, onDismiss = { newSection = null }) { name ->
            val id = model.createSection(name)
            if (id != null && chatId.isNotEmpty()) model.move(chatId, id)
        }
    }
    renameSection?.let { section ->
        TextPromptDialog("Rename section", section.name, "Rename", ZIcons.Rename, onDismiss = { renameSection = null }) { name ->
            model.renameSection(section.id, name)
        }
    }
    if (reorder && ws != null) ReorderPinsSheet(model, ws.front.pinned) { reorder = false }
}

/** What a session row can do, shared by every group on the page. */
class RowActions(
    val model: AppModel,
    val onOpen: (String) -> Unit,
    /** Archive (T3: settle); false when it didn't land. */
    val archive: (SessionRow) -> Boolean,
    /** User sections a row can move to (id, name). */
    val sections: List<Pair<String, String>>,
    /** Prompt for a new section, filing `chatId` into it (null: just create). */
    val newSection: (String?) -> Unit,
    /** The chat open beside the list (large screens). */
    val selected: String? = null,
    /** T3 Mode: rows get a visible actions button, a sheet, and snooze/settle swipes. */
    val mode: SidebarMode = SidebarMode.Default,
    /** "Remove animations": rows and shelves move without animating. */
    val reducedMotion: Boolean = false,
)

/** List item motion (fade and slide between groups and shelves), still when motion is off. */
fun LazyItemScope.itemMotion(reducedMotion: Boolean): Modifier =
    if (reducedMotion) Modifier.animateItem(fadeInSpec = null, placementSpec = null, fadeOutSpec = null) else Modifier.animateItem()

/** A T3 shelf: a folding bar and its rows when open. */
private fun LazyListScope.shelf(
    shelf: Shelf,
    rows: List<SessionRow>,
    total: Int,
    open: Set<String>,
    actions: RowActions,
    setOpen: (Boolean) -> Unit,
) {
    val isOpen = shelf.id in open
    item("shelf-${shelf.id}") {
        ShelfHeader(shelf, rows, total, isOpen, actions.reducedMotion, onToggle = { setOpen(!isOpen) }, modifier = itemMotion(actions.reducedMotion))
    }
    if (!isOpen) return
    itemsIndexed(rows, key = { _, r -> "${shelf.id}/${r.id}" }) { i, row ->
        Box(itemMotion(actions.reducedMotion).padding(horizontal = 16.dp).padding(bottom = ListItemDefaults.SegmentedGap)) {
            SwipeableSessionRow(row, i, rows.size, actions)
        }
    }
}

private fun LazyListScope.group(
    id: String,
    title: String?,
    rows: List<SessionRow>,
    actions: RowActions,
    collapsed: Boolean = false,
    onToggle: (() -> Unit)? = null,
    menu: List<MenuAction> = emptyList(),
) {
    if (title != null) {
        item("h-$id") {
            var open by remember { mutableStateOf(false) }
            Box(itemMotion(actions.reducedMotion)) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .combinedClickable(
                            onClick = { onToggle?.invoke() },
                            onLongClick = if (menu.isNotEmpty()) ({ open = true }) else null,
                        )
                        .padding(start = 28.dp, end = 12.dp, top = 20.dp, bottom = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                        Text(title, style = MaterialTheme.typography.titleSmallEmphasized, color = MaterialTheme.colorScheme.primary, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                        Spacer(Modifier.width(8.dp))
                        Text("${rows.size}", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    if (menu.isNotEmpty()) {
                        IconButton(onClick = { open = true }, modifier = Modifier.size(32.dp)) {
                            ZIcon(ZIcons.More, "$title options", Modifier.size(18.dp))
                        }
                    }
                    if (onToggle != null) {
                        IconButton(onClick = onToggle, modifier = Modifier.size(32.dp)) {
                            ZIcon(if (collapsed) ZIcons.ChevronDown else ZIcons.ChevronUp, if (collapsed) "Expand" else "Collapse", Modifier.size(18.dp))
                        }
                    }
                }
                Box(Modifier.align(Alignment.TopEnd).padding(end = 12.dp, top = 20.dp)) { ActionMenu(open, { open = false }, menu) }
            }
        }
    } else {
        item("h-$id") { Spacer(Modifier.height(8.dp)) }
    }
    if (collapsed) return
    itemsIndexed(rows, key = { _, r -> "$id/${r.id}" }) { i, row ->
        Box(itemMotion(actions.reducedMotion).padding(horizontal = 16.dp).padding(bottom = ListItemDefaults.SegmentedGap)) {
            SwipeableSessionRow(row, i, rows.size, actions)
        }
    }
}

/** What a swipe does, by direction; `run` is false when the write didn't land. */
private class SwipeAction(val label: String, val icon: Int, val run: () -> Boolean, val keepsRow: Boolean)

/**
 * Default: swipe right to pin (or unpin), left to archive.
 * T3: right to snooze (the sheet's lengths; a snoozed row picks a new one),
 * left to settle; a settled row swipes left to unsettle. Sending to a
 * snoozed row is what brings it back.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun SwipeableSessionRow(row: SessionRow, index: Int, count: Int, actions: RowActions) {
    val state = rememberSwipeToDismissBoxState()
    val scope = rememberCoroutineScope()
    // The lazy list keeps a removed row's saved swipe state under its key, so
    // a row brought back (Undo, unsettle) would return still swiped away.
    LaunchedEffect(Unit) {
        if (state.currentValue != SwipeToDismissBoxValue.Settled) state.snapTo(SwipeToDismissBoxValue.Settled)
    }
    val shape = segmentedShapes(index, count).shape
    val sheet = remember { mutableStateOf(false) }
    val model = actions.model
    val mode = actions.mode
    val (right, left) = when {
        !mode.isT3 -> SwipeAction(if (row.pinned) "Unpin" else "Pin", ZIcons.Pin, { model.setPinned(row.id, !row.pinned); true }, keepsRow = true) to
            SwipeAction("Archive", ZIcons.Archive, { actions.archive(row) }, keepsRow = false)
        row.archived -> null to SwipeAction(mode.unsettle, ZIcons.Unarchive, { model.unarchive(row.id) }, keepsRow = false)
        else -> SwipeAction("Snooze", ZIcons.Snooze, { sheet.value = true; true }, keepsRow = true) to
            SwipeAction(mode.settle, ZIcons.Settle, { actions.archive(row) }, keepsRow = false)
    }
    SwipeToDismissBox(
        state = state,
        enableDismissFromStartToEnd = right != null,
        onDismiss = {
            val action = when (it) {
                SwipeToDismissBoxValue.StartToEnd -> right
                SwipeToDismissBoxValue.EndToStart -> left
                else -> null
            } ?: return@SwipeToDismissBox
            // A failed write slides the row back rather than leaving a gap.
            if (!action.run() || action.keepsRow) scope.launch { state.reset() }
        },
        backgroundContent = {
            val fromStart = state.dismissDirection == SwipeToDismissBoxValue.StartToEnd
            val action = (if (fromStart) right else left) ?: return@SwipeToDismissBox
            // The glyph swells once the swipe would commit.
            val armed = state.targetValue != SwipeToDismissBoxValue.Settled
            val swell by animateFloatAsState(
                if (armed) 1.2f else 1f,
                if (actions.reducedMotion) snap() else MaterialTheme.motionScheme.fastSpatialSpec<Float>(),
                label = "swipe-swell",
            )
            val container = if (fromStart) MaterialTheme.colorScheme.secondaryContainer else MaterialTheme.colorScheme.tertiaryContainer
            val content = if (fromStart) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onTertiaryContainer
            Surface(Modifier.fillMaxSize(), shape = shape, color = container, contentColor = content) {
                Row(
                    Modifier.fillMaxSize().padding(horizontal = 28.dp),
                    horizontalArrangement = if (fromStart) Arrangement.Start else Arrangement.End,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    ZIcon(action.icon, null, Modifier.scale(swell))
                    // T3 names its swipes: snooze and settle aren't the pin and archive people know.
                    if (mode.isT3) {
                        Spacer(Modifier.width(10.dp))
                        Text(action.label, style = MaterialTheme.typography.labelLarge)
                    }
                }
            }
        },
    ) {
        SessionItem(row, index, count, actions, sheet)
    }
}

/** The list card colour: paper-white in light, raised in dark. */
@Composable
fun cardColor() = if (LocalDarkTheme.current) MaterialTheme.colorScheme.surfaceContainer else MaterialTheme.colorScheme.surfaceContainerLowest

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SessionItem(
    row: SessionRow,
    index: Int,
    count: Int,
    actions: RowActions,
    /** T3's actions sheet (a snooze swipe opens it too). */
    sheet: androidx.compose.runtime.MutableState<Boolean> = remember { mutableStateOf(false) },
) {
    var menu by remember { mutableStateOf(false) }
    var moving by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var customizing by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    val model = actions.model
    val mode = actions.mode
    Box {
        SegmentedListItem(
            onClick = { actions.onOpen(row.id) },
            onLongClick = {
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                if (mode.isT3) sheet.value = true else menu = true
            },
            shapes = segmentedShapes(index, count),
            colors = ListItemDefaults.segmentedColors(
                containerColor = if (row.id == actions.selected) MaterialTheme.colorScheme.secondaryContainer else cardColor(),
            ),
            leadingContent = { HarnessTile(row) },
            supportingContent = { Subline(row) },
            trailingContent = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        when {
                            row.snoozed -> SnoozeBadge(row)
                            row.archived -> Text(row.timeLabel, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.outline)
                            else -> StatusLabel(row)
                        }
                        row.pullRequest?.let { PrBadge(it.number, it.state) }
                    }
                    // Touch has no hover: T3's actions stay one visible tap away.
                    if (mode.isT3) {
                        IconButton(onClick = { sheet.value = true }, modifier = Modifier.padding(start = 4.dp).size(40.dp)) {
                            ZIcon(ZIcons.More, "Actions for ${row.title}", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            },
        ) {
            Text(
                row.title,
                style = if (row.unseen) MaterialTheme.typography.titleMediumEmphasized else MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        ActionMenu(
            menu,
            { menu = false },
            listOfNotNull(
                MenuAction(if (row.pinned) "Unpin" else "Pin", ZIcons.Pin) { model.setPinned(row.id, !row.pinned) },
                MenuAction("Move to section…", ZIcons.Folder) { moving = true },
                MenuAction("Rename…", ZIcons.Rename) { renaming = true },
                MenuAction(mode.settle, ZIcons.Archive, destructive = true) { actions.archive(row) },
            ),
        )
        ChoiceMenu(
            moving,
            { moving = false },
            listOf(
                MenuSection("Move to section", actions.sections.map { (id, name) ->
                    MenuChoice(name, row.sectionId == id, leading = { ZIcon(ZIcons.Folder, null, Modifier.size(20.dp)) }) { model.move(row.id, id) }
                } + listOf(
                    MenuChoice("No section", row.sectionId == null, leading = { ZIcon(ZIcons.Home, null, Modifier.size(20.dp)) }) { model.move(row.id, null) },
                    MenuChoice("New section…", false, leading = { ZIcon(ZIcons.Plus, null, Modifier.size(20.dp)) }) { actions.newSection(row.id) },
                )),
            ),
        )
    }
    if (sheet.value) {
        SessionActionsSheet(
            row,
            mode,
            SheetCallbacks(
                pin = { model.setPinned(row.id, !row.pinned) },
                move = { moving = true },
                rename = { renaming = true },
                settle = { actions.archive(row) },
                unsettle = { model.unarchive(row.id) },
                snooze = { model.snooze(row.id, it) },
                pickDateTime = { customizing = true },
            ),
            onDismiss = { sheet.value = false },
        )
    }
    if (customizing) SnoozeDateTimeDialogs(onDismiss = { customizing = false }) { model.snooze(row.id, it) }
    if (renaming) RenameDialog(row.title, onDismiss = { renaming = false }) { model.rename(row.id, it) }
}

/** The harness mark on a tile toned by the session's project. */
@Composable
fun HarnessTile(row: SessionRow) {
    val tone = ProjectColors.color(row.colorIndex(), LocalDarkTheme.current)
    Box(
        Modifier.size(48.dp).clip(RoundedCornerShape(16.dp)).background(tone.copy(alpha = if (LocalDarkTheme.current) 0.18f else 0.12f)),
        contentAlignment = Alignment.Center,
    ) { HarnessMark(row.harness, 24.dp, tint = MaterialTheme.colorScheme.onSurface) }
}

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
            ZIcon(ZIcons.Branch, null, Modifier.size(14.dp), tint = muted.copy(alpha = 0.7f))
            Spacer(Modifier.width(3.dp))
            Text(branch, style = MaterialTheme.typography.bodyMedium, color = muted.copy(alpha = 0.7f), maxLines = 1, overflow = TextOverflow.Ellipsis)
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
            ZIcon(ZIcons.PullRequest, null, Modifier.size(13.dp))
            Spacer(Modifier.width(3.dp))
            Text("$number", style = MaterialTheme.typography.labelMediumEmphasized)
        }
    }
}

@Composable
fun RenameDialog(current: String, onDismiss: () -> Unit, onRename: (String) -> Unit) =
    TextPromptDialog("Rename session", current, "Rename", ZIcons.Rename, onDismiss, onRename)

/** A one-field dialog (rename a session, name or rename a section). */
@Composable
fun TextPromptDialog(title: String, current: String, confirm: String, icon: Int, onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var text by remember { mutableStateOf(current) }
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    androidx.compose.runtime.LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { ZIcon(icon, null) },
        title = { Text(title) },
        text = {
            OutlinedTextField(
                text, { text = it },
                singleLine = true,
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.focusRequester(focus),
            )
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(text.trim()); onDismiss() }, enabled = text.isNotBlank()) { Text(confirm) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** Pins are an ordered list (synced to the desktop): move them up and down. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ReorderPinsSheet(model: AppModel, pinned: List<SessionRow>, onDismiss: () -> Unit) {
    val sheet = androidx.compose.material3.rememberModalBottomSheetState(skipPartiallyExpanded = true)
    androidx.compose.material3.ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Text("Reorder pinned", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.padding(start = 24.dp, bottom = 16.dp))
        LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            itemsIndexed(pinned, key = { _, r -> r.id }) { i, row ->
                SegmentedListItem(
                    onClick = {},
                    shapes = segmentedShapes(i, pinned.size),
                    colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                    leadingContent = { HarnessTile(row) },
                    trailingContent = {
                        Row {
                            IconButton(onClick = { model.movePin(row.id, pinned.getOrNull(i - 2)?.id, pinned[i - 1].id) }, enabled = i > 0) {
                                ZIcon(ZIcons.ChevronUp, "Move up", Modifier.size(20.dp))
                            }
                            IconButton(onClick = { model.movePin(row.id, pinned[i + 1].id, pinned.getOrNull(i + 2)?.id) }, enabled = i < pinned.size - 1) {
                                ZIcon(ZIcons.ChevronDown, "Move down", Modifier.size(20.dp))
                            }
                        }
                    },
                    modifier = Modifier.animateItem().padding(bottom = ListItemDefaults.SegmentedGap),
                ) { Text(row.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
        }
    }
}

@Composable
private fun EmptyState(filter: Filter, mode: SidebarMode, modifier: Modifier = Modifier) {
    val inboxZero = mode.isT3 && filter == Filter.All
    Column(modifier.fillMaxWidth().padding(horizontal = 48.dp, vertical = if (inboxZero) 32.dp else 48.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        if (inboxZero) {
            // Inbox zero earns the expressive cookie, not a muted glyph.
            Box(
                Modifier.size(64.dp).clip(androidx.compose.material3.MaterialShapes.Cookie9Sided.toShape()).background(MaterialTheme.colorScheme.tertiaryContainer),
                contentAlignment = Alignment.Center,
            ) { ZIcon(ZIcons.Settle, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.onTertiaryContainer) }
        } else {
            ZIcon(ZIcons.Chat, null, Modifier.size(40.dp), tint = MaterialTheme.colorScheme.outline)
        }
        Spacer(Modifier.height(12.dp))
        Text(
            when (filter) {
                Filter.All -> if (inboxZero) "Inbox zero" else "No sessions yet"
                Filter.NeedsYou -> "Nothing needs you"
                Filter.Working -> "No agents working"
                Filter.Pinned -> "Nothing pinned"
            },
            style = MaterialTheme.typography.titleLarge,
        )
        if (inboxZero) {
            Spacer(Modifier.height(4.dp))
            Text(
                "Running and snoozed sessions wait on the shelves below",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
        }
    }
}
