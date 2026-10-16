package sh.zeron.android.voice

import android.Manifest
import android.content.Context
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import android.util.Log
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.zeron_core.CodexDictationException
import uniffi.zeron_core.CodexDictationStatus
import uniffi.zeron_core.CoreClient
import uniffi.zeron_core.DeviceView
import uniffi.zeron_core.DictationAudioFormat
import uniffi.zeron_core.codexDictationHostCapability
import java.io.File
import java.util.UUID

/** Where composer dictation is. */
sealed interface DictationPhase {
    data object Idle : DictationPhase

    /** [startedAt] is `SystemClock.elapsedRealtime()`. */
    data class Recording(val startedAt: Long) : DictationPhase

    data object Transcribing : DictationPhase
}

/** What [host][DictationController.host] last said about lending its credential. */
data class DictationHostStatus(val hostId: String, val status: CodexDictationStatus)

/**
 * Codex dictation in the composer (desktop `dictation::codex`): this phone
 * records, the voice host (Settings → Voice) lends its ChatGPT credential
 * over the relay, and the core uploads the recording straight to ChatGPT.
 * The text lands in the draft at the cursor; nothing is ever sent.
 *
 * Off until the user turns it on. Availability is checked when Settings →
 * Voice opens, the app returns to the foreground or the host changes; never
 * polled.
 *
 * Main-thread only. Owned by `AppModel` beside [VoiceController].
 */
class DictationController(
    context: Context,
    private val client: StateFlow<CoreClient?>,
    internal val voice: VoiceController,
    /** The shell's snackbar: a message, an optional action label and its handler. */
    private val toast: (String, String?, (() -> Unit)?) -> Unit,
) {
    companion object {
        private const val TAG = "ZeronDictation"

        /** ChatGPT transcribes up to a minute (desktop "Up to 1 minute"). */
        const val MAX_MILLIS = 60_000
        val capability: String by lazy { codexDictationHostCapability() }

        /** Whether [device] lends its ChatGPT credential for dictation. */
        fun lends(device: DeviceView): Boolean = capability in device.capabilities
    }

    private val context = context.applicationContext
    private val scope = MainScope()
    private val prefs = this.context.getSharedPreferences("voice", 0)
    private val folder = File(this.context.cacheDir, "dictation")

    private val _enabled = MutableStateFlow(false)
    /** The Settings switch, per user and organization. */
    val enabled: StateFlow<Boolean> = _enabled.asStateFlow()

    /** The voice host (the remembered or only one online), when it lends credentials for dictation. */
    val host: StateFlow<DeviceView?> = combine(voice.devices, voice.selectedHost) { _, _ -> voice.startHost()?.takeIf(::lends) }
        .stateIn(scope, SharingStarted.Eagerly, voice.startHost()?.takeIf(::lends))

    private val _status = MutableStateFlow<DictationHostStatus?>(null)
    /** The last check, for whichever host it asked; compare its id with [host]. */
    val status: StateFlow<DictationHostStatus?> = _status.asStateFlow()

    /** The composer offers the microphone: switched on, and the host can lend right now. */
    val ready: StateFlow<Boolean> = combine(_enabled, host, _status) { enabled, host, status ->
        enabled && host != null && status?.hostId == host.id && status.status.available
    }.stateIn(scope, SharingStarted.Eagerly, false)

    private val _phase = MutableStateFlow<DictationPhase>(DictationPhase.Idle)
    val phase: StateFlow<DictationPhase> = _phase.asStateFlow()

    private val _permissionRequest = MutableStateFlow<CompletableDeferred<Boolean>?>(null)
    /** A pending microphone prompt for [DictationHost] to show. */
    val permissionRequest: StateFlow<CompletableDeferred<Boolean>?> = _permissionRequest.asStateFlow()

    /** The composer dictating (its draft gets the text), its inserter, and the host chosen at the start. */
    private var owner: Any? = null
    private var insert: ((String) -> Unit)? = null
    private var hostId: String? = null
    private var recorder: Recorder? = null
    private var work: Job? = null
    private var generation = 0L
    private var check: Job? = null
    private var checking: String? = null
    private val attached = mutableMapOf<Any, Int>()

    init {
        // A recording left behind by a process that died mid-dictation.
        folder.listFiles()?.forEach { it.delete() }
        // The switch belongs to one user in one organization; a new client starts over.
        scope.launch {
            client.collect {
                cancel()
                check?.cancel()
                checking = null
                _status.value = null
                _enabled.value = prefs.getBoolean(preferenceKey, false)
                if (_enabled.value) refresh()
            }
        }
        scope.launch { host.map { it?.id }.distinctUntilChanged().collect { if (_enabled.value) refresh() } }
        // A call takes the microphone: what was said so far still lands in the draft.
        scope.launch { voice.snapshot.map { it.live }.distinctUntilChanged().collect { live -> if (live) finish() } }
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                if (_enabled.value) refresh()
            }

            // Capture stops with the app in the background; transcribe what was said.
            override fun onStop(owner: LifecycleOwner) = finish()
        })
    }

    private val preferenceKey: String
        get() {
            val c = client.value
            val user = runCatching { c?.userId() }.getOrNull() ?: "none"
            val org = runCatching { c?.orgId() }.getOrNull() ?: "none"
            return "codexDictation.$user.$org"
        }

    fun setEnabled(on: Boolean) {
        prefs.edit().putBoolean(preferenceKey, on).apply()
        _enabled.value = on
        if (on) refresh() else cancel()
    }

    /** Ask [host] again whether it can lend a credential (no upload). */
    fun refresh() {
        val client = client.value ?: return
        val host = host.value ?: return
        if (check?.isActive == true && checking == host.id) return
        check?.cancel()
        checking = host.id
        check = scope.launch {
            val status = try {
                client.codexDictationStatus(host.id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                Log.w(TAG, "status check failed", e)
                return@launch
            }
            _status.value = DictationHostStatus(host.id, status)
        }
    }

    /** Whether [owner]'s composer is the one dictating. */
    fun owns(owner: Any): Boolean = this.owner === owner

    /**
     * The composer's microphone: starts recording for [owner] (asking for the
     * microphone first), or stops and transcribes into [insert]. Ignored while
     * another composer dictates, the permission prompt is up, or a call runs.
     */
    fun toggle(owner: Any, insert: (String) -> Unit) {
        when (_phase.value) {
            DictationPhase.Idle -> start(owner, insert)
            is DictationPhase.Recording -> if (owns(owner)) finish()
            DictationPhase.Transcribing -> Unit
        }
    }

    private fun start(owner: Any, insert: (String) -> Unit) {
        if (work?.isActive == true || voice.live || !ready.value) return
        val client = client.value ?: return
        val host = host.value ?: return
        val generation = ++generation
        this.owner = owner
        this.insert = insert
        hostId = host.id
        work = scope.launch {
            val granted = requestMicrophone()
            if (generation != this@DictationController.generation) return@launch
            if (!granted) {
                reset()
                toast("Zeron Nova needs microphone access to dictate.", "Settings") { openAppSettings(context) }
                return@launch
            }
            // A call may have started while the prompt was up.
            if (voice.live) return@launch reset()
            val recorder = Recorder.start(context, File(folder, "${UUID.randomUUID()}.m4a"), onLimit = ::finish) {
                if (generation == this@DictationController.generation) failed("Recording stopped unexpectedly. Your draft is safe.")
            }
            if (recorder == null) {
                reset()
                toast("The microphone isn't available right now.", null, null)
                return@launch
            }
            this@DictationController.recorder = recorder
            _phase.value = DictationPhase.Recording(SystemClock.elapsedRealtime())
            // The credential is fetched while the user talks.
            scope.launch { runCatching { client.prepareCodexDictation(host.id) } }
        }
    }

    /** Stop recording and transcribe into the dictating draft (tap, the minute limit, a call, the background). */
    fun finish() {
        if (_phase.value !is DictationPhase.Recording) return
        val recorder = recorder ?: return
        val client = client.value
        val hostId = hostId
        val insert = insert
        val generation = generation
        this.recorder = null
        _phase.value = DictationPhase.Transcribing
        work = scope.launch {
            val audio = recorder.finish()
            if (generation != this@DictationController.generation) return@launch
            if (audio == null) {
                reset()
                toast("No speech detected", null, null)
                return@launch
            }
            if (client == null || hostId == null || insert == null) return@launch reset()
            val text = try {
                client.transcribeCodexDictation(hostId, audio, DictationAudioFormat.M4A)
            } catch (e: CancellationException) {
                throw e
            } catch (e: CodexDictationException.Failed) {
                return@launch failed(e.reason)
            } catch (e: Exception) {
                Log.w(TAG, "transcription failed", e)
                return@launch failed("ChatGPT couldn't transcribe this recording. Your draft is safe.")
            }
            if (generation != this@DictationController.generation) return@launch
            reset()
            if (text.isBlank()) toast("No speech detected", null, null) else insert(text.trim())
        }
    }

    /** Drop the recording or the transcription in flight; the draft is untouched. */
    fun cancel() {
        generation++
        work?.cancel()
        work = null
        recorder?.discard()
        recorder = null
        reset()
    }

    private fun failed(message: String) {
        cancel()
        toast(message, null, null)
    }

    private fun reset() {
        owner = null
        insert = null
        hostId = null
        _phase.value = DictationPhase.Idle
    }

    /**
     * A composer's dictation controls came on screen. Their place moves as
     * the composer folds and unfolds, so [detach] waits a frame before
     * deciding the composer is really gone (its dictation is then cancelled).
     */
    internal fun attach(owner: Any) {
        attached[owner] = (attached[owner] ?: 0) + 1
    }

    internal fun detach(owner: Any) {
        val count = (attached[owner] ?: 1) - 1
        if (count > 0) attached[owner] = count else attached.remove(owner)
        scope.launch {
            if (owner !in attached && owns(owner)) cancel()
        }
    }

    // ── Microphone permission ──────────────────────────────────────────────

    private suspend fun requestMicrophone(): Boolean {
        if (voice.hasMicrophone()) return true
        val request = CompletableDeferred<Boolean>()
        _permissionRequest.value?.complete(false)
        _permissionRequest.value = request
        return try {
            request.await() && voice.hasMicrophone()
        } finally {
            if (_permissionRequest.value === request) _permissionRequest.value = null
        }
    }

    internal fun completePermission(granted: Boolean) {
        _permissionRequest.value?.complete(granted)
        _permissionRequest.value = null
    }
}

/**
 * One recording: AAC in MPEG-4 at 16 kHz mono into a cache file, stopped by
 * the minute limit at the latest. The file is read back once and deleted;
 * audio is never logged.
 */
private class Recorder private constructor(private val recorder: MediaRecorder, private val file: File) {
    companion object {
        /** Null when the microphone can't be opened (another app holds it, no hardware). */
        fun start(context: Context, file: File, onLimit: () -> Unit, onError: () -> Unit): Recorder? {
            if (androidx.core.content.ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED) return null
            file.parentFile?.mkdirs()
            val recorder = if (Build.VERSION.SDK_INT >= 31) {
                MediaRecorder(context)
            } else {
                @Suppress("DEPRECATION")
                MediaRecorder()
            }
            val started = Recorder(recorder, file)
            return try {
                recorder.setAudioSource(MediaRecorder.AudioSource.MIC)
                recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
                recorder.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
                recorder.setAudioSamplingRate(16_000)
                recorder.setAudioChannels(1)
                recorder.setAudioEncodingBitRate(32_000)
                recorder.setMaxDuration(DictationController.MAX_MILLIS)
                recorder.setOutputFile(file)
                recorder.setOnInfoListener { _, what, _ ->
                    if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED) {
                        started.limitReached = true
                        onLimit()
                    }
                }
                recorder.setOnErrorListener { _, _, _ -> onError() }
                recorder.prepare()
                recorder.start()
                started
            } catch (e: Exception) {
                Log.w("ZeronDictation", "recorder failed to start", e)
                started.discard()
                null
            }
        }
    }

    /** The minute limit already stopped the recorder. */
    var limitReached = false

    /** Finishes the file and returns its bytes, or null when nothing usable was recorded. */
    suspend fun finish(): ByteArray? {
        // Stopping right after starting fails: no audio was captured.
        val stopped = runCatching { recorder.stop() }.isSuccess || limitReached
        recorder.release()
        return withContext(Dispatchers.IO) {
            try {
                if (stopped) file.readBytes().takeIf { it.isNotEmpty() } else null
            } catch (e: Exception) {
                null
            } finally {
                file.delete()
            }
        }
    }

    fun discard() {
        runCatching { recorder.stop() }
        recorder.release()
        file.delete()
    }
}
