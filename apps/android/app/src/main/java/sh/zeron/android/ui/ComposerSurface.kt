package sh.zeron.android.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.foundation.BorderStroke
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** What the composer's trailing button does right now. */
enum class ComposerAction { Send, Queue, Stop }

/**
 * The composer, shared by sessions and the new-session page. One tonal
 * surface with two states: a resting one-line capsule — [+] Message… [↑] —
 * and, while focused or holding a draft, a card with the full-width prompt
 * above a toolbar of context chips.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ComposerSurface(
    text: String,
    onText: (String) -> Unit,
    placeholder: String,
    action: ComposerAction,
    onAction: () -> Unit,
    modifier: Modifier = Modifier,
    alwaysCard: Boolean = false,
    focusRequester: FocusRequester = remember { FocusRequester() },
    chips: @Composable RowScope.() -> Unit = {},
) {
    var focused by remember { mutableStateOf(false) }
    val card = alwaysCard || focused || text.isNotEmpty()
    Surface(
        shape = RoundedCornerShape(28.dp),
        color = composerContainer(),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = modifier.fillMaxWidth(),
    ) {
        Column(Modifier.animateContentSize(MaterialTheme.motionScheme.defaultSpatialSpec())) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = 56.dp)) {
                if (!card) {
                    Spacer(Modifier.width(4.dp))
                    AttachButton()
                } else {
                    Spacer(Modifier.width(20.dp))
                }
                Box(Modifier.weight(1f).padding(vertical = if (card) 16.dp else 8.dp)) {
                    if (text.isEmpty()) {
                        Text(placeholder, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
                    }
                    BasicTextField(
                        text,
                        onText,
                        textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                        maxLines = if (card) 8 else 2,
                        modifier = Modifier
                            .fillMaxWidth()
                            .focusRequester(focusRequester)
                            .onFocusChanged { focused = it.isFocused },
                    )
                }
                if (!card) {
                    Spacer(Modifier.width(8.dp))
                    ActionButton(action, text.isNotBlank(), onAction)
                    Spacer(Modifier.width(8.dp))
                } else {
                    Spacer(Modifier.width(16.dp))
                }
            }
            AnimatedVisibility(card, enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically()) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(start = 4.dp, end = 8.dp, bottom = 8.dp),
                ) {
                    AttachButton()
                    Row(
                        Modifier
                            .weight(1f)
                            .graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }
                            .drawWithContent {
                                drawContent()
                                // Chips scrolling under the send button fade out.
                                val fade = 24.dp.toPx()
                                drawRect(
                                    Brush.horizontalGradient(listOf(Color.Black, Color.Transparent), startX = size.width - fade, endX = size.width),
                                    topLeft = androidx.compose.ui.geometry.Offset(size.width - fade, 0f),
                                    blendMode = BlendMode.DstIn,
                                )
                            }
                            .horizontalScroll(rememberScrollState()),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        content = chips,
                    )
                    Spacer(Modifier.width(8.dp))
                    ActionButton(action, text.isNotBlank(), onAction)
                }
            }
        }
    }
}

@Composable
private fun AttachButton() {
    // Attachments land with the picker; the affordance stays where it will live.
    IconButton(onClick = {}, enabled = false) {
        sh.zeron.android.design.ZIcon(sh.zeron.android.design.ZIcons.Plus, "Attach", Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f))
    }
}

/** Send / queue / stop: one control whose shape morphs on press. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ActionButton(action: ComposerAction, hasText: Boolean, onClick: () -> Unit) {
    val haptics = LocalHapticFeedback.current
    AnimatedContent(
        action == ComposerAction.Stop && !hasText,
        transitionSpec = { (scaleIn() + fadeIn()) togetherWith (scaleOut() + fadeOut()) },
        label = "action",
    ) { stop ->
        if (stop) {
            FilledTonalIconButton(
                onClick = {
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                    onClick()
                },
                shapes = IconButtonDefaults.shapes(),
                colors = IconButtonDefaults.filledTonalIconButtonColors(
                    containerColor = MaterialTheme.colorScheme.errorContainer,
                    contentColor = MaterialTheme.colorScheme.onErrorContainer,
                ),
                modifier = Modifier.size(40.dp),
            ) { sh.zeron.android.design.ZIcon(sh.zeron.android.design.ZIcons.Stop, "Stop", Modifier.size(18.dp)) }
        } else {
            FilledIconButton(
                onClick = {
                    haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                    onClick()
                },
                enabled = hasText,
                shapes = IconButtonDefaults.shapes(),
                colors = IconButtonDefaults.filledIconButtonColors(
                    disabledContainerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    disabledContentColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                ),
                modifier = Modifier.size(40.dp),
            ) { sh.zeron.android.design.ZIcon(sh.zeron.android.design.ZIcons.Send, if (action == ComposerAction.Queue) "Queue" else "Send", Modifier.size(20.dp)) }
        }
    }
}

/** A context chip in the composer toolbar; `menu` is its (lazily built) choice menu. */
@Composable
fun ContextChip(
    label: String,
    leading: @Composable () -> Unit,
    onClick: () -> Unit,
    tint: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    menu: @Composable () -> Unit = {},
) {
    Box {
        Surface(
            onClick = onClick,
            shape = RoundedCornerShape(50),
            color = chipContainer(),
            contentColor = tint,
        ) {
            Row(
                Modifier.heightIn(min = 34.dp).padding(start = 10.dp, end = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Box(Modifier.size(16.dp), contentAlignment = Alignment.Center) { leading() }
                Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        menu()
    }
}

class MenuChoice(
    val label: String,
    val selected: Boolean = false,
    val supporting: String? = null,
    val leading: (@Composable () -> Unit)? = null,
    val onClick: () -> Unit,
)

class MenuSection(val title: String?, val choices: List<MenuChoice>)

/** An expressive menu: labelled, segmented groups of selectable items. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ChoiceMenu(expanded: Boolean, onDismiss: () -> Unit, sections: List<MenuSection>) {
    DropdownMenuPopup(expanded = expanded, onDismissRequest = onDismiss) {
        val groups = sections.filter { it.choices.isNotEmpty() }
        groups.forEachIndexed { g, section ->
            DropdownMenuGroup(shapes = MenuDefaults.groupShape(g, groups.size)) {
                section.title?.let { title ->
                    MenuDefaults.Label { Text(title, style = MaterialTheme.typography.labelMedium) }
                }
                section.choices.forEachIndexed { i, choice ->
                    DropdownMenuItem(
                        selected = choice.selected,
                        onClick = {
                            choice.onClick()
                            onDismiss()
                        },
                        text = { Text(choice.label) },
                        supportingText = choice.supporting?.let { { Text(it) } },
                        shapes = MenuDefaults.itemShape(i, section.choices.size),
                        leadingIcon = choice.leading,
                        selectedLeadingIcon = { sh.zeron.android.design.ZIcon(sh.zeron.android.design.ZIcons.Check, null, Modifier.size(20.dp)) },
                        colors = MenuDefaults.selectableItemColors(
                            selectedContainerColor = MaterialTheme.colorScheme.secondaryContainer,
                            selectedTextColor = MaterialTheme.colorScheme.onSecondaryContainer,
                            selectedLeadingIconColor = MaterialTheme.colorScheme.onSecondaryContainer,
                        ),
                    )
                }
            }
            if (g < groups.size - 1) Spacer(Modifier.size(MenuDefaults.GroupSpacing))
        }
    }
}

class MenuAction(val label: String, @androidx.annotation.DrawableRes val icon: Int, val destructive: Boolean = false, val onClick: () -> Unit)

/** An expressive action menu (one segmented group). */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ActionMenu(expanded: Boolean, onDismiss: () -> Unit, actions: List<MenuAction>) {
    DropdownMenuPopup(expanded = expanded, onDismissRequest = onDismiss) {
        DropdownMenuGroup(shapes = MenuDefaults.groupShape(0, 1)) {
            actions.forEachIndexed { i, a ->
                val tint = if (a.destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
                DropdownMenuItem(
                    onClick = {
                        onDismiss()
                        a.onClick()
                    },
                    text = { Text(a.label, color = tint) },
                    shape = MenuDefaults.itemShape(i, actions.size).shape,
                    leadingIcon = { sh.zeron.android.design.ZIcon(a.icon, null, Modifier.size(20.dp), tint = tint) },
                )
            }
        }
    }
}
