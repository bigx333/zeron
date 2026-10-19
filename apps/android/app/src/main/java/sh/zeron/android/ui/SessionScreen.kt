package sh.zeron.android.ui

import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import android.widget.Toast
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.animation.animateColorAsState
import sh.zeron.android.design.ZIcon
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsPadding
import sh.zeron.android.design.ZIcons
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Archive
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.graphics.ImageBitmap
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.foundation.background
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import sh.zeron.android.core.AppModel
import sh.zeron.android.transcript.ImageCache
import sh.zeron.android.design.Geist
import sh.zeron.android.design.GeistMono
import sh.zeron.android.transcript.Transcript
import sh.zeron.android.transcript.TranscriptActions
import sh.zeron.android.transcript.TranscriptState
import uniffi.zeron_core.ComposerState
import uniffi.zeron_core.ConnectivityState
import uniffi.zeron_core.CoreException
import uniffi.zeron_core.SendState
import uniffi.zeron_core.SessionHandle

private data class TextSheet(val title: String, val text: String, val mono: Boolean)

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SessionScreen(model: AppModel, chatId: String, onBack: () -> Unit, onOpenChat: (String) -> Unit = {}, onOpenVoice: () -> Unit = {}, onVoiceSettings: () -> Unit = {}) {
    val client by model.client.collectAsState()
    val core = client ?: return
    val handle: SessionHandle = remember(chatId) { runCatching { core.openSession(chatId) }.getOrNull() } ?: run {
        // Never a stand-in transcript: back to Sessions with a note.
        LaunchedEffect(chatId) {
            model.toast("Couldn't open session. Try again.")
            onBack()
        }
        return
    }
    val transcript = remember(chatId) { TranscriptState() }
    DisposableEffect(chatId) {
        transcript.engine.attach(core, chatId)
        handle.setViewAttached(true)
        model.markSeen(chatId)
        onDispose {
            handle.setViewAttached(false)
            transcript.close()
            // What arrived while it was open has been seen too.
            model.markSeen(chatId)
        }
    }

    // Composer-facing state: re-pulled on this chat's events.
    var composer by remember { mutableStateOf(handle.composer()) }
    val workspace by model.workspace.collectAsState()
    val connectivity by model.connectivity.collectAsState()
    LaunchedEffect(chatId) {
        model.sessionEvents.collect { if (it == chatId) composer = handle.composer() }
    }
    // Upload rings on pending thumbnails follow the escort.
    LaunchedEffect(composer.transferProgress) { transcript.uploadProgress = composer.transferProgress }
    val row = remember(workspace, chatId) { model.row(chatId) }

    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var sheet by remember { mutableStateOf<TextSheet?>(null) }
    var viewing by remember { mutableStateOf<ImageBitmap?>(null) }
    val actions = remember(chatId, composer.host.deviceId) {
        val device = composer.host.deviceId
        suspend fun fetch(ref: String): Result<ImageBitmap> {
            val key = ImageCache.key(device, ref)
            ImageCache[key]?.let { return Result.success(it) }
            return runCatching {
                val bytes = core.readAttachment(device, ref)
                withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }
                    ?: error("It isn't an image this phone can show.")
            }.onFailure { if (it is CancellationException) throw it }.onSuccess { ImageCache.put(key, it) }
        }
        suspend fun load(ref: String): ImageBitmap? = fetch(ref).getOrNull()
        fun cantOpen(what: String, why: String? = null) =
            Toast.makeText(context, listOfNotNull("Couldn't open $what.", why).joinToString(" "), Toast.LENGTH_LONG).show()
        val open: (String) -> Unit = { ref ->
            scope.launch {
                fetch(ref).onSuccess { viewing = it }.onFailure { cantOpen(ref.substringAfterLast('/'), it.hostReason) }
            }
        }
        TranscriptActions(
            openUrl = { url ->
                val uri = Uri.parse(url)
                when (uri.scheme?.lowercase()) {
                    "http", "https" -> CustomTabsIntent.Builder().build().launchUrl(context, uri)
                    // An image chip opens its upload full size, as its thumbnail did.
                    "zeron-preview" -> uri.getQueryParameter("ref")?.let(open) ?: cantOpen(url)
                    // A path on the chat's host: only it can read the file, so fetch it from there.
                    null, "file" -> {
                        val local = uri.authority.isNullOrEmpty() ||
                            (uri.scheme != null && uri.authority.equals("localhost", ignoreCase = true))
                        val path = if (uri.scheme == null && local) url else uri.path
                        path?.takeIf { local && it.startsWith("/") && '\u0000' !in it }?.let(open) ?: cantOpen(url)
                    }
                    else -> runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, uri)) }.onFailure { cantOpen(url) }
                }
            },
            loadImage = ::load,
            cachedImage = { ref -> ImageCache[ImageCache.key(device, ref)] },
            showText = { title, text, mono -> sheet = TextSheet(title, text, mono) },
            openImage = open,
        )
    }
    // The composer's draft outlives the panel that replaces it while the agent asks.
    val draft = remember(chatId) { ComposerModel(model.drafts.load(chatId)) }

    val project = row?.project?.name ?: "No project"
    val subtitle = composer.host.name?.let { "$project @ $it" } ?: project
    var orchestration by remember(chatId) { mutableStateOf(false) }
    val related = remember(workspace?.revision, chatId) { core.childSessions(parentId = chatId) }
    var overflow by remember { mutableStateOf(false) }
    var snoozing by remember { mutableStateOf(false) }
    var pickingWake by remember { mutableStateOf(false) }
    val mode by model.sidebarMode.collectAsState()
    val clipboard = LocalClipboardManager.current

    // Flat Material chrome: the header sits on the page and takes the
    // container tone once the transcript scrolls under it; the transcript
    // ends where the composer begins (nothing streams behind it).
    val scrolled by remember { derivedStateOf { transcript.offset > 1f } }
    val headerColor by animateColorAsState(
        if (scrolled) MaterialTheme.colorScheme.surfaceContainerHigh else MaterialTheme.colorScheme.background,
        MaterialTheme.motionScheme.defaultEffectsSpec(),
        label = "header",
    )
    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
        Surface(color = headerColor) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .statusBarsPadding()
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TonalCircleButton(ZIcons.Back, "Back", onClick = onBack, container = MaterialTheme.colorScheme.surfaceContainerHighest)
                Column(Modifier.weight(1f).padding(horizontal = 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(composer.title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMediumEmphasized)
                    Text(subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                // The session has its own composer; the call stays a tap away up here.
                if (sh.zeron.android.voice.rememberVoiceVisible(model.voice)) {
                    sh.zeron.android.voice.VoiceHeaderButton(model.voice, onOpenStage = onOpenVoice, onOpenSettings = onVoiceSettings)
                    androidx.compose.foundation.layout.Spacer(Modifier.size(8.dp))
                }
                TonalCircleButton(ZIcons.Branch, "Tasks and side chats", onClick = { orchestration = true }, container = MaterialTheme.colorScheme.surfaceContainerHighest)
                androidx.compose.foundation.layout.Spacer(Modifier.size(8.dp))
                Box {
                    TonalCircleButton(ZIcons.More, "More", onClick = { overflow = true }, container = MaterialTheme.colorScheme.surfaceContainerHighest)
                    ActionMenu(
                        overflow,
                        { overflow = false },
                        listOfNotNull(
                            MenuAction("Copy transcript", ZIcons.Copy) { transcript.frame?.let { clipboard.setText(AnnotatedString(it.plainText())) } },
                            row?.let { r -> MenuAction(if (r.pinned) "Unpin" else "Pin", ZIcons.Pin) { model.setPinned(chatId, !r.pinned) } },
                            // T3: snooze lengths in a sheet (touch, no hover); a snoozed chat wakes from here.
                            row?.takeIf { mode.isT3 && !it.archived }?.let { MenuAction(if (it.snoozed) "Snooze again…" else "Snooze…", ZIcons.Snooze) { snoozing = true } },
                            row?.let { r ->
                                if (r.archived) {
                                    MenuAction(mode.unsettle, ZIcons.Unarchive) { model.unarchive(chatId) }
                                } else {
                                    // Back to the list only once it's really gone from it.
                                    MenuAction(mode.settle, if (mode.isT3) ZIcons.Settle else ZIcons.Archive) { if (model.archive(chatId)) onBack() }
                                }
                            },
                        ),
                    )
                }
            }
        }
        (row?.task?.parentChatId ?: row?.fork?.sourceChatId ?: row?.parentChatId)?.let { parentId ->
            val parent = remember(workspace?.revision, parentId) { core.sessionRow(chatId = parentId) }
            androidx.compose.material3.TextButton(onClick = { onOpenChat(parentId) }, enabled = parent != null, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                sh.zeron.android.design.ZIcon(ZIcons.Back, null, Modifier.size(16.dp))
                androidx.compose.foundation.layout.Spacer(Modifier.size(8.dp))
                Text("${if (row?.task != null) "Task from" else if (row?.fork != null) "Forked from" else "Side chat of"} ${parent?.title ?: "parent chat"}", maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Transcript(transcript, actions, Modifier.fillMaxSize())
            val showJump by remember { derivedStateOf { !transcript.following && transcript.distanceFromBottom > 400f } }
            androidx.compose.animation.AnimatedVisibility(
                showJump,
                enter = fadeIn() + scaleIn(),
                exit = fadeOut() + scaleOut(),
                modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 12.dp),
            ) {
                SmallFloatingActionButton(
                    onClick = { transcript.scrollToBottom() },
                    shape = CircleShape,
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                ) { ZIcon(ZIcons.ArrowDown, "Jump to latest", Modifier.size(22.dp)) }
            }
        }
        Column(Modifier.fillMaxWidth().imePadding().navigationBarsPadding()) {
            Banner(composer, connectivity?.state, onRetry = { runCatching { handle.retryDelivery() } })
            val input = composer.openInput
            // The panel replaces the composer while the agent asks.
            if (input != null) {
                QuestionPanel(input) { answers -> runCatching { handle.respondInput(input.requestId, answers) } }
            } else {
                Composer(model, core, handle, composer, row, transcript, draft)
            }
        }
    }

    viewing?.let { ImageViewer(it) { viewing = null } }

    if (snoozing) SnoozeSheet(row, onDismiss = { snoozing = false }, onPickDateTime = { pickingWake = true }) { model.snooze(chatId, it) }
    if (pickingWake) SnoozeDateTimeDialogs(onDismiss = { pickingWake = false }) { model.snooze(chatId, it) }

    if (orchestration) {
        OrchestrationSheet(core, row, related, onDismiss = { orchestration = false }, onOpen = { target -> orchestration = false; onOpenChat(target) })
    }
    sheet?.let { s ->
        ModalBottomSheet(onDismissRequest = { sheet = null }) {
            Text(s.title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp))
            SelectionContainer {
                Text(
                    s.text,
                    fontFamily = if (s.mono) GeistMono else Geist,
                    style = if (s.mono) MaterialTheme.typography.bodySmall else MaterialTheme.typography.bodyLarge,
                    modifier = Modifier
                        .fillMaxWidth()
                        .verticalScroll(rememberScrollState())
                        .padding(24.dp),
                )
            }
        }
    }
}

/** Why a core call failed, in the host's words rather than the binding's `reason=…`. */
private val Throwable.hostReason: String?
    get() = when (this) {
        is CoreException.NotFound -> reason
        is CoreException.InvalidArgument -> reason
        is CoreException.HostUnavailable -> reason
        is CoreException.Unsupported -> reason
        is CoreException.HostException -> reason
        is CoreException.Network -> reason
        is CoreException.Auth -> reason
        is CoreException.Storage -> reason
        is CoreException.NotImplemented -> reason
        is CoreException.Internal -> reason
        is CoreException.Closed -> null
        else -> message
    }

/** The status pill above the composer (iOS SessionChrome banner). */
@Composable
private fun Banner(c: ComposerState, connectivity: ConnectivityState?, onRetry: () -> Unit) {
    // Reconnect countdowns tick each second.
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    val retryAt = c.room.retryAtMs
    if (!c.room.connected && retryAt != null) {
        LaunchedEffect(retryAt) {
            while (true) {
                now = System.currentTimeMillis()
                delay(1_000)
            }
        }
    }
    val text = when {
        c.sendState == SendState.FAILED -> "Not delivered"
        c.sendState == SendState.QUEUED -> "${c.host.name ?: "Host"} is offline — will send when it's back"
        c.sendState == SendState.SENDING -> "Sending to host…"
        connectivity == ConnectivityState.OFFLINE -> "Offline — sends are saved"
        !c.room.connected && retryAt != null -> "Reconnecting in ${maxOf(1L, (retryAt - now) / 1000)}s"
        else -> null
    } ?: return
    StatusBanner(text, if (c.sendState == SendState.FAILED) "Retry" to onRetry else null)
}
