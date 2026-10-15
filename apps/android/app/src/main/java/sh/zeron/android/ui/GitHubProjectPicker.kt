package sh.zeron.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import sh.zeron.android.core.AppModel
import sh.zeron.android.design.ZIcon
import sh.zeron.android.design.ZIcons
import uniffi.zeron_core.DeviceView
import uniffi.zeron_core.GitHubRepository

/** Account and checkout both belong to [device], never to the phone. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun GitHubProjectPicker(
    model: AppModel,
    device: DeviceView,
    onBusyChanged: (Boolean) -> Unit,
    onCreated: (String) -> Unit,
) {
    val client by model.client.collectAsState()
    val scope = rememberCoroutineScope()
    val focus = LocalFocusManager.current
    var repositories by remember { mutableStateOf<List<GitHubRepository>>(emptyList()) }
    var login by remember { mutableStateOf<String?>(null) }
    var cursor by remember { mutableStateOf<String?>(null) }
    var query by remember { mutableStateOf("") }
    var selected by remember { mutableStateOf<GitHubRepository?>(null) }
    var loading by remember { mutableStateOf(false) }
    var creating by remember { mutableStateOf(false) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var cloneError by remember { mutableStateOf<String?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    DisposableEffect(Unit) { onDispose { onBusyChanged(false) } }

    LaunchedEffect(attempt) {
        val current = client ?: return@LaunchedEffect
        loading = true
        loadError = null
        val seen = mutableSetOf<String>()
        try {
            do {
                val page = current.listGithubRepositories(device.id, cursor)
                currentCoroutineContext().ensureActive()
                if (model.client.value !== current) return@LaunchedEffect
                if (login != null && login != page.login) {
                    repositories = emptyList()
                    selected = null
                    login = null
                    cursor = null
                    loadError = "GitHub account changed. Retry to reload repositories."
                    break
                }
                login = page.login
                repositories = (repositories + page.repositories).distinctBy { it.nameWithOwner }
                cursor = page.nextCursor
                if (cursor != null && !seen.add(cursor!!)) {
                    loadError = "GitHub returned a repeated page. Retry to continue."
                    break
                }
            } while (cursor != null)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            loadError = githubProjectFailure(error, device.name)
        } finally {
            loading = false
        }
    }

    val filtered = repositories.filter { it.nameWithOwner.contains(query.trim(), ignoreCase = true) }
    Column(Modifier.fillMaxSize().imePadding().padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            label = { Text("Search repositories") },
            leadingIcon = { ZIcon(ZIcons.Search, null, Modifier.size(20.dp)) },
            singleLine = true,
            enabled = !creating,
            modifier = Modifier.fillMaxWidth().testTag("github-repository-search"),
        )
        Text(
            login?.let { "Signed in as $it on ${device.name}" } ?: "Uses the GitHub CLI account on ${device.name}",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        LazyColumn(Modifier.weight(1f).testTag("github-repositories"), contentPadding = PaddingValues(bottom = 12.dp)) {
            if (loadError != null) item("error") {
                Column {
                    Text(loadError!!, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                    TextButton(onClick = { attempt++ }, enabled = !creating, modifier = Modifier.testTag("github-retry")) { Text("Retry") }
                }
            }
            if (filtered.isEmpty()) item("empty") {
                Text(
                    when {
                        loading -> "Loading repositories…"
                        repositories.isEmpty() && loadError == null -> "No repositories found for this account."
                        repositories.isNotEmpty() -> "No repositories match your search."
                        else -> ""
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(vertical = 16.dp),
                )
            }
            itemsIndexed(filtered, key = { _, repo -> repo.nameWithOwner }) { index, repo ->
                SegmentedListItem(
                    onClick = { selected = repo; cloneError = null },
                    enabled = !creating,
                    shapes = segmentedShapes(index, filtered.size),
                    colors = ListItemDefaults.segmentedColors(containerColor =
                        if (selected?.nameWithOwner == repo.nameWithOwner) MaterialTheme.colorScheme.secondaryContainer else cardColor()),
                    leadingContent = { ZIcon(ZIcons.Branch, null, Modifier.size(22.dp)) },
                    trailingContent = { if (selected?.nameWithOwner == repo.nameWithOwner) ZIcon(ZIcons.Check, "Selected", Modifier.size(20.dp)) },
                    modifier = Modifier.padding(bottom = ListItemDefaults.SegmentedGap).testTag("github-repo-${repo.nameWithOwner}"),
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(repo.nameWithOwner, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        repo.description?.takeIf { it.isNotBlank() }?.let {
                            Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        }
                        if (repo.isPrivate) Text("Private", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            if (loading && repositories.isNotEmpty()) item("loading") { LoadingIndicator(Modifier.size(24.dp).padding(top = 4.dp)) }
        }
        cloneError?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
        Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                if (creating) "Cloning on ${device.name} and adding the project…" else "The checkout is saved on ${device.name}.",
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Button(
                enabled = selected != null && !creating && client != null,
                shapes = ButtonDefaults.shapes(),
                contentPadding = PaddingValues(horizontal = 24.dp, vertical = 16.dp),
                modifier = Modifier.fillMaxWidth().testTag("clone-github-project"),
                onClick = {
                    val repo = selected ?: return@Button
                    val current = client ?: return@Button
                    focus.clearFocus()
                    creating = true
                    onBusyChanged(true)
                    cloneError = null
                    scope.launch {
                        try {
                            val id = current.cloneGithubProject(device.id, repo.nameWithOwner)
                            currentCoroutineContext().ensureActive()
                            if (model.client.value === current) {
                                model.refreshWorkspace()
                                onCreated(id)
                            }
                        } catch (cancelled: CancellationException) {
                            throw cancelled
                        } catch (error: Exception) {
                            cloneError = githubProjectFailure(error, device.name)
                        } finally {
                            creating = false
                            onBusyChanged(false)
                        }
                    }
                },
            ) {
                if (creating) LoadingIndicator(Modifier.size(20.dp)) else Text(selected?.let { "Clone ${it.nameWithOwner.substringAfter('/')}" } ?: "Clone project")
            }
        }
    }
}

private fun githubProjectFailure(error: Exception, host: String): String {
    val message = error.message ?: "Couldn't access GitHub."
    return when {
        message.contains("not installed", true) || message.contains("not authenticated", true) ->
            "$message Install GitHub CLI and run gh auth login on $host, then retry."
        message.contains("unknown method", true) || message.contains("not supported", true) ->
            "Update Zeron on $host to use GitHub projects."
        else -> message
    }
}
