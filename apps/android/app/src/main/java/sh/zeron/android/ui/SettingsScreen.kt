package sh.zeron.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.DarkMode
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.LightMode
import androidx.compose.material.icons.outlined.Palette
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.SettingsSuggest
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LargeFlexibleTopAppBar
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.ToggleButtonDefaults
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ThemeMode
import uniffi.zeron_core.coreVersion

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsScreen(model: AppModel) {
    val appearance by model.appearance.collectAsState()
    val workspace by model.workspace.collectAsState()
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            LargeFlexibleTopAppBar(
                title = { Text("Settings") },
                scrollBehavior = scroll,
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
            )
        },
    ) { padding ->
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = padding.calculateTopPadding(), bottom = 32.dp)) {
            item {
                // Account card.
                Surface(
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.primaryContainer,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Row(Modifier.padding(20.dp), verticalAlignment = Alignment.CenterVertically) {
                        Box(
                            Modifier.size(52.dp).clip(MaterialShapes.Sunny.toShape()).background(MaterialTheme.colorScheme.primary),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                model.accountName.take(1).uppercase(),
                                style = MaterialTheme.typography.titleLarge,
                                color = MaterialTheme.colorScheme.onPrimary,
                            )
                        }
                        Spacer(Modifier.width(16.dp))
                        Column {
                            Text(model.accountName, style = MaterialTheme.typography.titleLarge, color = MaterialTheme.colorScheme.onPrimaryContainer)
                            Text(model.accountDetail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.8f))
                        }
                    }
                }
            }
            item { SectionHeader("Appearance") }
            item {
                SegmentedGroup(2) { i, shape ->
                    Surface(shape = shape, color = MaterialTheme.colorScheme.surfaceContainerLowest) {
                        when (i) {
                            0 -> Column(Modifier.padding(16.dp)) {
                                Text("Theme", style = MaterialTheme.typography.titleMedium)
                                Spacer(Modifier.height(12.dp))
                                // Connected button group.
                                Row(horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween)) {
                                    val modes = listOf(
                                        Triple(ThemeMode.System, "System", Icons.Outlined.SettingsSuggest),
                                        Triple(ThemeMode.Light, "Light", Icons.Outlined.LightMode),
                                        Triple(ThemeMode.Dark, "Dark", Icons.Outlined.DarkMode),
                                    )
                                    modes.forEachIndexed { index, (mode, label, icon) ->
                                        ToggleButton(
                                            checked = appearance.mode == mode,
                                            onCheckedChange = { model.setAppearance(appearance.copy(mode = mode)) },
                                            modifier = Modifier.weight(1f).semantics { role = Role.RadioButton },
                                            shapes = when (index) {
                                                0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                                                modes.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                                                else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
                                            },
                                        ) {
                                            Icon(icon, null, Modifier.size(ToggleButtonDefaults.IconSize))
                                            Spacer(Modifier.size(ToggleButtonDefaults.IconSpacing))
                                            Text(label)
                                        }
                                    }
                                }
                            }
                            else -> ListItem(
                                headlineContent = { Text("Wallpaper colors") },
                                supportingContent = { Text("Tint the app with your device's Material You palette") },
                                leadingContent = { Icon(Icons.Outlined.Palette, null) },
                                trailingContent = {
                                    Switch(appearance.dynamicColor, { model.setAppearance(appearance.copy(dynamicColor = it)) })
                                },
                                colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest),
                            )
                        }
                    }
                }
            }
            val devices = workspace?.devices.orEmpty()
            if (devices.isNotEmpty()) {
                item { SectionHeader("Devices") }
                item {
                    SegmentedGroup(devices.size) { i, shape ->
                        val device = devices[i]
                        ListItem(
                            headlineContent = { Text(device.name) },
                            supportingContent = {
                                Text(
                                    listOfNotNull(
                                        if (device.isSelf) "This device" else if (device.online) "Online" else "Offline",
                                        device.version?.let { "v$it" },
                                        if (device.sessionCount > 0u) "${device.sessionCount} sessions" else null,
                                    ).joinToString(" · "),
                                )
                            },
                            leadingContent = {
                                Icon(if (device.isExecutionHost) Icons.Outlined.Computer else Icons.Outlined.PhoneAndroid, null)
                            },
                            trailingContent = {
                                Box(
                                    Modifier.size(10.dp).clip(CircleShape).background(
                                        if (device.online || device.isSelf) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.outlineVariant,
                                    ),
                                )
                            },
                            modifier = Modifier.clip(shape),
                            colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest),
                        )
                    }
                }
            }
            item { SectionHeader("About") }
            item {
                SegmentedGroup(2) { i, shape ->
                    if (i == 0) {
                        ListItem(
                            headlineContent = { Text("Version") },
                            supportingContent = { Text("Zeron for Android · core ${coreVersion()}") },
                            leadingContent = { Icon(Icons.Outlined.Info, null) },
                            modifier = Modifier.clip(shape),
                            colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest),
                        )
                    } else {
                        ListItem(
                            headlineContent = { Text(if (model.isDemo) "Leave demo" else "Sign out", color = MaterialTheme.colorScheme.error) },
                            leadingContent = { Icon(Icons.AutoMirrored.Outlined.Logout, null, tint = MaterialTheme.colorScheme.error) },
                            modifier = Modifier.clip(shape).clickable { model.signOut() },
                            colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLowest),
                        )
                    }
                }
            }
        }
    }
}
