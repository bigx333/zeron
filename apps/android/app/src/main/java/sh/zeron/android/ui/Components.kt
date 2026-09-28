package sh.zeron.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import sh.zeron.android.design.LocalDarkTheme
import sh.zeron.android.design.ProjectColors
import uniffi.zeron_core.ChatIndicator
import uniffi.zeron_core.SessionRow
import uniffi.zeron_core.projectColorIndex

/**
 * Grouped-list corners (Android 16 settings style): the group's outer
 * corners are large, the seams between items small.
 */
fun segmentShape(index: Int, count: Int, outer: Dp = 24.dp, inner: Dp = 6.dp): Shape {
    val top = if (index == 0) outer else inner
    val bottom = if (index == count - 1) outer else inner
    return RoundedCornerShape(topStart = top, topEnd = top, bottomStart = bottom, bottomEnd = bottom)
}

/** A section title above a group. */
@Composable
fun SectionHeader(text: String, modifier: Modifier = Modifier, trailing: @Composable (() -> Unit)? = null) {
    Box(modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 20.dp, bottom = 8.dp)) {
        Text(
            text,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.align(Alignment.CenterStart),
        )
        trailing?.let { Box(Modifier.align(Alignment.CenterEnd)) { it() } }
    }
}

/**
 * A group of items with segmented corners and 2dp seams.
 */
@Composable
fun SegmentedGroup(
    count: Int,
    modifier: Modifier = Modifier,
    item: @Composable (index: Int, shape: Shape) -> Unit,
) {
    Column(modifier.padding(horizontal = 12.dp)) {
        for (i in 0 until count) {
            item(i, segmentShape(i, count))
            if (i < count - 1) Spacer(Modifier.height(2.dp))
        }
    }
}

/** Project monogram tile, toned by the core's stable project color. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ProjectTile(name: String?, colorIndex: Int, size: Dp = 40.dp, modifier: Modifier = Modifier) {
    val tone = ProjectColors.color(colorIndex, LocalDarkTheme.current)
    // Expressive shapes: projects are "cookies", projectless sessions a circle.
    val shape = if (name != null) MaterialShapes.Cookie9Sided.toShape() else CircleShape
    Box(
        modifier.size(size).clip(shape).background(tone.copy(alpha = 0.16f)),
        contentAlignment = Alignment.Center,
    ) {
        if (name != null) {
            Text(
                name.trim().take(1).uppercase(),
                color = tone,
                fontWeight = FontWeight.SemiBold,
                fontSize = (size.value * 0.42f).sp,
            )
        } else {
            Icon(Icons.Outlined.Home, null, Modifier.size(size * 0.5f), tint = tone)
        }
    }
}

fun SessionRow.colorIndex(): Int = (project?.colorIndex ?: projectColorIndex("home")).toInt()

/** Live status at the trailing edge of a session row. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun StatusIndicator(indicator: ChatIndicator, unseen: Boolean) {
    when (indicator) {
        ChatIndicator.WORKING -> LoadingIndicator(Modifier.size(28.dp))
        ChatIndicator.AWAITING_INPUT -> Surface(
            shape = RoundedCornerShape(50),
            color = MaterialTheme.colorScheme.tertiaryContainer,
            contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
        ) {
            Text(
                "Needs you",
                style = MaterialTheme.typography.labelSmall,
                modifier = Modifier.padding(PaddingValues(horizontal = 8.dp, vertical = 3.dp)),
            )
        }
        ChatIndicator.ERRORED -> Icon(Icons.Filled.ErrorOutline, "Failed", Modifier.size(20.dp), tint = MaterialTheme.colorScheme.error)
        else -> if (unseen) Box(Modifier.size(10.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary))
    }
}
