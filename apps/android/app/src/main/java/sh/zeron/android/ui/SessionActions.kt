package sh.zeron.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.core.Shelf
import sh.zeron.android.core.SidebarMode
import sh.zeron.android.core.SnoozeOption
import sh.zeron.android.core.SnoozeOptions
import sh.zeron.android.core.snoozeCaption
import sh.zeron.android.design.HarnessMark
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.SessionRow

/** What the actions sheet hands back to the row (menus anchored there, dialogs owned there). */
class SheetCallbacks(
    val pin: () -> Unit,
    val move: () -> Unit,
    val rename: () -> Unit,
    val settle: () -> Unit,
    val unsettle: () -> Unit,
    val snooze: (SnoozeOption) -> Unit,
    val wake: () -> Unit,
)

/**
 * A session's T3 actions as a bottom sheet — the touch home of everything the
 * desktop puts on hover: snooze lengths up top as big tap targets, Wake early
 * for a snoozed row, then pin / move / rename / settle.
 */
@Composable
fun SessionActionsSheet(row: SessionRow, mode: SidebarMode, callbacks: SheetCallbacks, onDismiss: () -> Unit) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    // Let the sheet slide away before the row moves shelves under it.
    fun then(action: () -> Unit) {
        scope.launch { sheet.hide() }.invokeOnCompletion {
            onDismiss()
            action()
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            Row(Modifier.padding(start = 8.dp, bottom = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                HarnessTile(row)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(row.title, style = MaterialTheme.typography.titleLargeEmphasized, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(
                        row.project?.name ?: row.deviceName ?: "No project",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (!row.archived) {
                if (row.snoozed) {
                    SnoozedCard(row) {
                        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                        then(callbacks.wake)
                    }
                    Spacer(Modifier.height(16.dp))
                }
                Text(
                    if (row.snoozed) "Snooze again" else "Snooze",
                    style = MaterialTheme.typography.titleSmallEmphasized,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(start = 12.dp, bottom = 10.dp),
                )
                SnoozeChoices { option ->
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                    then { callbacks.snooze(option) }
                }
                Spacer(Modifier.height(20.dp))
            }
            val items = buildList {
                if (row.archived) {
                    add(SheetItem(mode.unsettle, ZIcons.Unarchive) { then(callbacks.unsettle) })
                    add(SheetItem("Rename…", ZIcons.Rename) { then(callbacks.rename) })
                } else {
                    add(SheetItem(if (row.pinned) "Unpin" else "Pin", ZIcons.Pin) { then(callbacks.pin) })
                    add(SheetItem("Move to section…", ZIcons.Folder) { then(callbacks.move) })
                    add(SheetItem("Rename…", ZIcons.Rename) { then(callbacks.rename) })
                    add(SheetItem(mode.settle, if (mode.isT3) ZIcons.Settle else ZIcons.Archive, accent = true) { then(callbacks.settle) })
                }
            }
            Column(verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                items.forEachIndexed { i, item ->
                    SegmentedListItem(
                        onClick = item.onClick,
                        shapes = segmentedShapes(i, items.size),
                        colors = ListItemDefaults.segmentedColors(containerColor = cardColor()),
                        leadingContent = {
                            if (item.accent) {
                                IconTile(item.icon, MaterialTheme.colorScheme.tertiaryContainer, MaterialTheme.colorScheme.onTertiaryContainer)
                            } else {
                                IconTile(item.icon)
                            }
                        },
                    ) { Text(item.label) }
                }
            }
        }
    }
}

/** Just the snooze lengths, from the open session's header menu. */
@Composable
fun SnoozeSheet(row: SessionRow?, onDismiss: () -> Unit, onSnooze: (SnoozeOption) -> Unit) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainerLow) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
            Text("Snooze", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.padding(start = 8.dp))
            Text(
                if (row?.snoozed == true) "Wakes ${snoozeCaption(row.snoozeLabel, row.snoozedUntilMs)} — pick a new time" else "Out of the inbox until it wakes",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 8.dp, top = 2.dp, bottom = 16.dp),
            )
            SnoozeChoices { option ->
                haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                scope.launch { sheet.hide() }.invokeOnCompletion {
                    onDismiss()
                    onSnooze(option)
                }
            }
        }
    }
}

private class SheetItem(val label: String, val icon: Int, val accent: Boolean = false, val onClick: () -> Unit)

/** The five snooze lengths as one row of shape-morphing tonal buttons. */
@Composable
fun SnoozeChoices(onPick: (SnoozeOption) -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (option in SnoozeOptions) {
            FilledTonalButton(
                onClick = { onPick(option) },
                shapes = ButtonDefaults.shapes(),
                contentPadding = PaddingValues(horizontal = 4.dp, vertical = 14.dp),
                modifier = Modifier
                    .weight(1f)
                    .heightIn(min = 56.dp)
                    .semantics { contentDescription = "Snooze ${option.label}" },
            ) { Text(option.short, style = MaterialTheme.typography.titleMediumEmphasized) }
        }
    }
}

/** "Snoozed · in 42m" with Wake now, atop the sheet for a snoozed row. */
@Composable
private fun SnoozedCard(row: SessionRow, onWake: () -> Unit) {
    Surface(shape = RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.secondaryContainer, contentColor = MaterialTheme.colorScheme.onSecondaryContainer) {
        Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            ZIcon(ZIcons.Snooze, null, Modifier.size(22.dp))
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text("Snoozed", style = MaterialTheme.typography.titleMediumEmphasized)
                Text("Wakes ${snoozeCaption(row.snoozeLabel, row.snoozedUntilMs)}", style = MaterialTheme.typography.bodyMedium)
            }
            androidx.compose.material3.Button(onClick = onWake, shapes = ButtonDefaults.shapes()) {
                ZIcon(ZIcons.Wake, null, Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text("Wake now")
            }
        }
    }
}

/** A snoozed row's trailing badge (in place of the live status). */
@Composable
fun SnoozeBadge(row: SessionRow) {
    val tone = MaterialTheme.colorScheme.secondary
    Surface(shape = RoundedCornerShape(8.dp), color = tone.copy(alpha = 0.12f), contentColor = tone) {
        Row(Modifier.padding(horizontal = 6.dp, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            ZIcon(ZIcons.Snooze, null, Modifier.size(13.dp))
            Spacer(Modifier.width(3.dp))
            Text(snoozeCaption(row.snoozeLabel, row.snoozedUntilMs), style = MaterialTheme.typography.labelMediumEmphasized, maxLines = 1)
        }
    }
}

/**
 * A T3 shelf: one tonal bar that folds its rows away. Folded, it peeks at
 * the harnesses inside; the chevron turns as it opens.
 */
@Composable
fun ShelfHeader(shelf: Shelf, rows: List<SessionRow>, total: Int, open: Boolean, reducedMotion: Boolean, onToggle: () -> Unit, modifier: Modifier = Modifier) {
    val turn by animateFloatAsState(
        if (open) 180f else 0f,
        if (reducedMotion) snap() else MaterialTheme.motionScheme.defaultSpatialSpec<Float>(),
        label = "shelf-chevron",
    )
    val (icon, caption) = when (shelf) {
        Shelf.Working -> ZIcons.Refresh to if (total == 0) "No agents running" else "$total running"
        Shelf.Snoozed -> ZIcons.Snooze to (rows.firstOrNull()?.let { "Next wakes ${snoozeCaption(it.snoozeLabel, it.snoozedUntilMs)}" } ?: "Nothing snoozed")
        Shelf.Settled -> ZIcons.Settle to if (total == 0) "Nothing settled" else "$total settled"
    }
    Surface(
        onClick = onToggle,
        shape = RoundedCornerShape(28.dp),
        color = chipContainer(),
        modifier = modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 8.dp)
            .semantics {
                role = Role.Button
                stateDescription = if (open) "Expanded" else "Collapsed"
            },
    ) {
        Row(Modifier.padding(start = 10.dp, end = 14.dp, top = 10.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(MaterialTheme.colorScheme.secondaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                // Agents at work keep moving even folded away (still when motion is off).
                if (shelf == Shelf.Working && total > 0 && !reducedMotion) {
                    LoadingIndicator(Modifier.size(30.dp), color = MaterialTheme.colorScheme.onSecondaryContainer)
                } else {
                    ZIcon(icon, null, Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSecondaryContainer)
                }
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(shelf.title, style = MaterialTheme.typography.titleMediumEmphasized, maxLines = 1)
                Text(caption, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            AnimatedVisibility(
                !open && rows.isNotEmpty(),
                enter = if (reducedMotion) fadeIn(snap()) else fadeIn() + expandHorizontally(expandFrom = Alignment.End),
                exit = if (reducedMotion) fadeOut(snap()) else fadeOut() + shrinkHorizontally(shrinkTowards = Alignment.End),
            ) { Peek(rows) }
            if (total > 0) {
                Box(
                    Modifier.padding(start = 8.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary.copy(alpha = 0.14f)).padding(horizontal = 8.dp, vertical = 2.dp),
                ) { Text("$total", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary) }
            }
            Spacer(Modifier.width(6.dp))
            ZIcon(ZIcons.ChevronDown, if (open) "Collapse ${shelf.title}" else "Expand ${shelf.title}", Modifier.size(20.dp).rotate(turn))
        }
    }
}

/** Up to three overlapping harness marks: what's on a folded shelf. */
@Composable
private fun Peek(rows: List<SessionRow>) {
    val shown = rows.distinctBy { it.harness }.take(3)
    val ring = chipContainer()
    Box(Modifier.padding(end = 2.dp).width((22 + 14 * (shown.size - 1)).dp).height(24.dp)) {
        shown.forEachIndexed { i, row ->
            Box(
                Modifier
                    .offset(x = (14 * i).dp)
                    .size(24.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.surfaceContainerLowest)
                    .border(2.dp, ring, CircleShape),
                contentAlignment = Alignment.Center,
            ) { HarnessMark(row.harness, 13.dp, tint = MaterialTheme.colorScheme.onSurface) }
        }
    }
}
