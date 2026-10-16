package sh.zeron.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.fadeIn
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
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
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
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
import kotlin.math.roundToInt

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
 * vendor's models are listed directly. Picking a model closes the sheet;
 * effort applies as the slider settles and the sheet stays open.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ModelSheet(
    sections: List<ModelSection>,
    effort: String?,
    efforts: List<String>,
    onEffort: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    fun close() {
        scope.launch { sheet.hide() }.invokeOnCompletion { if (!sheet.isVisible) onDismiss() }
    }
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
            Text(
                "Model",
                style = MaterialTheme.typography.headlineSmallEmphasized,
                modifier = Modifier.padding(start = 24.dp, bottom = 12.dp),
            )
            // The chosen model's effort; hidden while browsing a vendor, whose models have their own ladders.
            AnimatedVisibility(page == null && effort != null && efforts.size > 1, enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically()) {
                if (effort != null) EffortSlider(effort, efforts, onEffort, Modifier.padding(start = 24.dp, end = 24.dp, bottom = 12.dp))
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
                                    close()
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
                            close()
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ModelRows(section: ModelSection, models: List<ModelInfo>, describe: Boolean, onPick: (ModelInfo) -> Unit) {
    models.forEachIndexed { i, m ->
        val vendor = ModelVendor.of(m.id, section.harness)
        SheetRow(
            selected = section.selected(m),
            index = i,
            count = models.size,
            leading = { VendorMark(vendor, section.harness, 24.dp) },
            title = m.label,
            supporting = m.description?.takeIf { describe },
            trailing = if (section.selected(m)) {
                { SelectedBadge() }
            } else {
                null
            },
        ) { onPick(m) }
    }
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

/**
 * The model's effort levels as a stepped slider, low to high, with the level
 * named above it. Each step ticks; the choice applies when the thumb settles.
 */
@Composable
private fun EffortSlider(effort: String, efforts: List<String>, onEffort: (String) -> Unit, modifier: Modifier = Modifier) {
    val haptics = LocalHapticFeedback.current
    val last = efforts.size - 1
    var value by remember(efforts) { mutableFloatStateOf(efforts.indexOf(effort).coerceAtLeast(0).toFloat()) }
    // A change from elsewhere (another device, a model switch) moves the thumb.
    LaunchedEffect(effort, efforts) { value = efforts.indexOf(effort).coerceAtLeast(0).toFloat() }
    val step = value.roundToInt().coerceIn(0, last)
    Column(modifier) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Effort", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
            AnimatedContent(
                reasoningLabel(efforts[step]),
                transitionSpec = { fadeIn() togetherWith fadeOut() using SizeTransform(clip = false) },
                contentAlignment = Alignment.CenterEnd,
                label = "effort",
            ) { Text(it, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary) }
        }
        Slider(
            value = value,
            onValueChange = {
                if (it.roundToInt() != value.roundToInt()) haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
                value = it
            },
            valueRange = 0f..last.toFloat(),
            steps = (last - 1).coerceAtLeast(0),
            onValueChangeFinished = { onEffort(efforts[value.roundToInt().coerceIn(0, last)]) },
        )
        Row(horizontalArrangement = Arrangement.SpaceBetween, modifier = Modifier.fillMaxWidth()) {
            Text(reasoningLabel(efforts.first()), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(reasoningLabel(efforts.last()), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
