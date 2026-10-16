package sh.zeron.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import sh.zeron.android.design.ModelMark
import sh.zeron.android.design.ModelVendor
import sh.zeron.android.design.VendorMark
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.ModelInfo
import uniffi.zeron_core.reasoningLabel

/** One harness's models in a [ModelSheet]. */
class ModelSection(
    val title: String?,
    val harness: String?,
    val models: List<ModelInfo>,
    val selected: (ModelInfo) -> Boolean,
    val onPick: (ModelInfo) -> Unit,
)

/** Models grouped by vendor in picker order. */
private fun vendorGroups(models: List<ModelInfo>, harness: String?): List<Pair<ModelVendor, List<ModelInfo>>> {
    val groups = models.groupBy { ModelVendor.of(it.id, harness) }
    return ModelVendor.entries.mapNotNull { v -> groups[v]?.let { v to it } }
}

/**
 * The composer's one model chip: the chosen model's brand mark, its name and,
 * when the model has an effort ladder, the effort ("Fable 5 · High"). It
 * opens [ModelSheet], where both are picked.
 */
@Composable
fun ModelChip(model: String?, harness: String?, label: String, effort: String?, onClick: () -> Unit) {
    ContextChip(
        if (effort != null) "$label · ${reasoningLabel(effort)}" else label,
        leading = { ModelMark(model, harness, 14.dp) },
        onClick = onClick,
    )
}

/**
 * Model and effort in one sheet. Effort is a stepped slider over the chosen
 * model's levels; models keep the list short: with more than one vendor in
 * it, the vendors come first and each opens its models (back returns). One
 * vendor's models are listed directly. Picking a model returns to the top,
 * where the effort follows the new model; Done (or a swipe) closes the sheet.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ModelSheet(
    sections: List<ModelSection>,
    effort: String?,
    efforts: List<String>,
    onEffort: (String) -> Unit,
    onDismiss: () -> Unit,
    defaultEffort: String? = null,
) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    fun close() {
        scope.launch { sheet.hide() }.invokeOnCompletion { if (!sheet.isVisible) onDismiss() }
    }
    val haptics = LocalHapticFeedback.current
    fun pickedHaptic() = haptics.performHapticFeedback(HapticFeedbackType.Confirm)
    val grouped = sections.map { it to vendorGroups(it.models, it.harness) }
    val categories = grouped.sumOf { (_, groups) -> groups.size } > 1
    // Keyed by harness and vendor, not the section: a refreshed list brings new callbacks.
    var open by remember { mutableStateOf<Pair<String?, ModelVendor>?>(null) }
    val page = open?.let { (harness, vendor) ->
        sections.firstOrNull { it.harness == harness }
            ?.takeIf { s -> s.models.any { ModelVendor.of(it.id, harness) == vendor } }
            ?.let { it to vendor }
    }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheet,
        containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
    ) {
        BackHandler(enabled = page != null) { open = null }
        Column(Modifier.animateContentSize(MaterialTheme.motionScheme.defaultSpatialSpec())) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(start = 24.dp, end = 12.dp, bottom = 12.dp)) {
                Text("Model", style = MaterialTheme.typography.headlineSmallEmphasized, modifier = Modifier.weight(1f))
                TextButton(onClick = ::close) { Text("Done", style = MaterialTheme.typography.titleMedium) }
            }
            // The chosen model's effort; hidden while browsing a vendor, whose models have their own ladders.
            AnimatedVisibility(page == null && effort != null && efforts.size > 1, enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically()) {
                if (effort != null) EffortSlider(effort, efforts, defaultEffort, onEffort, Modifier.padding(start = 24.dp, end = 24.dp, bottom = 16.dp))
            }
            AnimatedContent(
                page,
                transitionSpec = {
                    // Forward into a vendor slides left; back slides right.
                    val forward = targetState != null
                    (slideInHorizontally { w -> if (forward) w / 4 else -w / 4 } + fadeIn()) togetherWith
                        (slideOutHorizontally { w -> if (forward) -w / 4 else w / 4 } + fadeOut()) using
                        SizeTransform(clip = false)
                },
                contentKey = { it?.let { (s, v) -> s.harness to v } },
                label = "model page",
            ) { shown ->
                Column(Modifier.verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
                    if (shown == null) {
                        grouped.forEach { (section, groups) ->
                            section.title?.let { SectionLabel(it) }
                            if (categories) {
                                groups.forEachIndexed { i, (vendor, list) ->
                                    val picked = list.firstOrNull(section.selected)
                                    val count = if (list.size == 1) "1 model" else "${list.size} models"
                                    SheetRow(
                                        selected = picked != null,
                                        index = i,
                                        count = groups.size,
                                        leading = { VendorMark(vendor, section.harness, 24.dp) },
                                        title = vendor.title,
                                        supporting = picked?.let { "${it.label} · $count" } ?: count,
                                        trailing = { ZIcon(ZIcons.ChevronRight, null, Modifier.size(20.dp)) },
                                    ) { open = section.harness to vendor }
                                }
                            } else {
                                ModelRows(section, section.models, describe = true) { m ->
                                    section.onPick(m)
                                    pickedHaptic()
                                }
                            }
                        }
                    } else {
                        val (section, vendor) = shown
                        val models = section.models.filter { ModelVendor.of(it.id, section.harness) == vendor }
                        // One vendor through two providers (Claude direct and via a gateway): name the provider.
                        val providers = models.mapTo(HashSet()) { it.id.substringBefore('/', "") }.size
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier.padding(start = 8.dp, end = 24.dp, bottom = 4.dp),
                        ) {
                            IconButton(onClick = { open = null }) { ZIcon(ZIcons.Back, "Back", Modifier.size(22.dp)) }
                            Spacer(Modifier.size(4.dp))
                            VendorMark(vendor, section.harness, 20.dp)
                            Spacer(Modifier.size(10.dp))
                            Column {
                                Text(vendor.title, style = MaterialTheme.typography.titleMedium)
                                section.title?.let {
                                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                        }
                        ModelRows(section, models, describe = providers > 1) { m ->
                            section.onPick(m)
                            pickedHaptic()
                            open = null
                        }
                    }
                }
            }
        }
    }
}

/**
 * A list of models, current ones first. Older generations ([ModelInfo.legacy])
 * fold behind a "More models" row that expands in place; it starts open when
 * the chosen model is one of them, so the selection is never hidden. A list
 * that is all current or all older has no fold.
 */
@Composable
private fun ModelRows(section: ModelSection, models: List<ModelInfo>, describe: Boolean, onPick: (ModelInfo) -> Unit) {
    val current = models.filterNot { it.legacy }
    val older = models.filter { it.legacy }
    val fold = current.isNotEmpty() && older.isNotEmpty()
    var more by remember(section.harness, older.map { it.id }) { mutableStateOf(older.any(section.selected)) }
    val shown = if (fold) current else models
    shown.forEachIndexed { i, m -> ModelRow(section, m, describe, i, shown.size + if (fold) 1 else 0, onPick) }
    if (!fold) return
    val turn by animateFloatAsState(if (more) 180f else 0f, MaterialTheme.motionScheme.fastSpatialSpec(), label = "more")
    SheetRow(
        selected = false,
        index = shown.size,
        count = shown.size + 1,
        leading = { ZIcon(ZIcons.More, null, Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant) },
        title = "More models",
        supporting = older.joinToString(", ") { it.label },
        trailing = { ZIcon(ZIcons.ChevronDown, if (more) "Show fewer" else "Show more", Modifier.size(20.dp).rotate(turn)) },
    ) { more = !more }
    AnimatedVisibility(
        more,
        enter = fadeIn() + expandVertically(MaterialTheme.motionScheme.defaultSpatialSpec()),
        exit = fadeOut() + shrinkVertically(MaterialTheme.motionScheme.fastSpatialSpec()),
    ) {
        Column(Modifier.padding(top = 8.dp)) {
            older.forEachIndexed { i, m -> ModelRow(section, m, describe, i, older.size, onPick) }
        }
    }
}

@Composable
private fun ModelRow(section: ModelSection, m: ModelInfo, describe: Boolean, index: Int, count: Int, onPick: (ModelInfo) -> Unit) {
    val picked = section.selected(m)
    SheetRow(
        selected = picked,
        index = index,
        count = count,
        leading = { VendorMark(ModelVendor.of(m.id, section.harness), section.harness, 24.dp) },
        title = m.label,
        supporting = m.description?.takeIf { describe },
        trailing = if (picked) {
            { SelectedBadge() }
        } else {
            null
        },
    ) { onPick(m) }
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 28.dp, top = 12.dp, bottom = 8.dp),
    )
}

@Composable
private fun SelectedBadge() {
    Box(
        Modifier.size(28.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary),
        contentAlignment = Alignment.Center,
    ) { ZIcon(ZIcons.Check, "Selected", Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onPrimary) }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun SheetRow(
    selected: Boolean,
    index: Int,
    count: Int,
    leading: @Composable () -> Unit,
    title: String,
    supporting: String?,
    trailing: (@Composable () -> Unit)?,
    onClick: () -> Unit,
) {
    SegmentedListItem(
        onClick = onClick,
        shapes = segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(
            containerColor = if (selected) MaterialTheme.colorScheme.secondaryContainer else cardColor(),
        ),
        leadingContent = { Box(Modifier.size(28.dp), contentAlignment = Alignment.Center) { leading() } },
        supportingContent = supporting?.let {
            { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        },
        trailingContent = trailing,
        modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = ListItemDefaults.SegmentedGap),
    ) { Text(title, style = MaterialTheme.typography.titleMedium) }
}
