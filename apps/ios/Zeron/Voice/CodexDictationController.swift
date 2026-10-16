import AVFoundation
import UIKit

/// Codex dictation: this iPhone records a clip, the voice host lends the
/// ChatGPT credential from its Codex sign-in, and the core uploads the clip
/// from this phone straight to ChatGPT (audio never crosses the relay). Off
/// until turned on in Voice settings. One dictation at a time, app-wide; the
/// composer that started it gets the words, and nothing is ever sent.
@MainActor
final class CodexDictationController: NSObject {
    static let capability = codexDictationHostCapability()
    /// One clip is at most a minute, like Codex dictation everywhere.
    static let maxDuration: TimeInterval = 60

    enum State: Equatable {
        case idle
        case recording(since: Date)
        case transcribing
    }

    /// How a dictation ended, for the composer that started it.
    enum Outcome {
        case text(String)
        case noSpeech
        case microphoneDenied
        /// User-facing; the draft was not touched.
        case failed(String)
    }

    private weak var app: AppModel?
    private(set) var state: State = .idle
    private var observers: [UUID: () -> Void] = [:]
    private var tokens: [AnyObject] = []
    private var notifications: [NSObjectProtocol] = []
    private var lastHost: String?
    private weak var lastClient: CoreClient?

    init(app: AppModel) {
        self.app = app
        super.init()
        lastHost = host?.id
        lastClient = app.client
        tokens.append(app.observe { [weak self] in self?.hostsChanged() })
        tokens.append(app.voice.observe { [weak self] in self?.voiceChanged() })
        let center = NotificationCenter.default
        notifications.append(center.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.foregrounded() }
        })
        // Nobody dictates from a pocket: keep what was said and transcribe it.
        notifications.append(center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.stop() }
        })
        // A phone call, Siri or an alarm takes the microphone: same.
        notifications.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] notification in
            guard (notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt) == AVAudioSession.InterruptionType.began.rawValue else { return }
            MainActor.assumeIsolated { self?.stop() }
        })
        if enabled { refreshStatus() }
    }

    // MARK: Preference and host

    private var preferenceKey: String {
        "codexDictation.\(app?.client?.userId() ?? "none").\(app?.client?.orgId() ?? "none")"
    }

    /// Off until the user turns it on in Voice settings.
    var enabled: Bool {
        get { UserDefaults.standard.bool(forKey: preferenceKey + ".enabled") }
        set {
            guard newValue != enabled else { return }
            UserDefaults.standard.set(newValue, forKey: preferenceKey + ".enabled")
            if newValue { refreshStatus() } else { cancel() }
            changed()
        }
    }

    static func compatible(_ device: DeviceView) -> Bool {
        device.isExecutionHost && device.capabilities.contains(capability)
    }

    /// The device a voice call would use, when it can lend a credential.
    var host: DeviceView? {
        guard let host = app?.voice.startHost, Self.compatible(host) else { return nil }
        return host
    }

    private func hostsChanged() {
        let id = host?.id
        let client = app?.client
        guard id != lastHost || client !== lastClient else { return }
        // A new account (or sign-out) never inherits the old one's dictation.
        if client !== lastClient, busy { cancel() }
        lastHost = id
        lastClient = client
        if enabled { refreshStatus() } else { changed() }
    }

    private func voiceChanged() {
        // A call takes the microphone and the audio session.
        if app?.voice.live == true, busy { cancel() }
        // The voice host is chosen there.
        hostsChanged()
    }

    private func foregrounded() {
        if enabled { refreshStatus() }
    }

    // MARK: Status

    private(set) var status: CodexDictationStatus?
    /// The host and account `status` describes.
    private var statusHost: String?
    private weak var statusClient: CoreClient?
    private var requestedHost: String?
    private weak var requestedClient: CoreClient?
    private var statusRequest: UInt64 = 0

    /// `status`, when it is about the current host and account.
    private var currentStatus: CodexDictationStatus? {
        guard let host, let client = app?.client, statusHost == host.id, statusClient === client else { return nil }
        return status
    }

    /// The composer offers the microphone: on, with a host that can lend.
    var offered: Bool { enabled && currentStatus?.available == true }

    /// Settings can turn it on: the host can lend right now.
    var canEnable: Bool { currentStatus?.available == true }

    /// Asks the host whether it can lend a credential (no upload). Never
    /// polled: Settings opening, the app returning and the host changing ask.
    func refreshStatus() {
        statusRequest &+= 1
        // Recorded before notifying, so `refreshStatusIfStale` settles.
        requestedHost = host?.id
        requestedClient = app?.client
        guard let client = app?.client, let host else { return changed() }
        let request = statusRequest
        let id = host.id
        Task { [weak self] in
            let status = await client.codexDictationStatus(hostDeviceId: id)
            guard let self, self.statusRequest == request else { return }
            self.status = status
            self.statusHost = id
            self.statusClient = client
            self.changed()
        }
    }

    /// Settings: ask again only when the host or account changed since.
    func refreshStatusIfStale() {
        guard host?.id != requestedHost || app?.client !== requestedClient else { return }
        refreshStatus()
    }

    /// The switch's explanation in Voice settings.
    var settingsDetail: String {
        guard let host else {
            if let voiceHost = app?.voice.startHost {
                return "Update Zeron Nova on \(voiceHost.name) to use Codex dictation."
            }
            return app?.voice.hosts.isEmpty ?? true
                ? "Needs a Mac or server that's online."
                : "Choose a device above to use Codex dictation."
        }
        guard let status = currentStatus else { return "Checking \(host.name)…" }
        if status.available {
            return "Transcribed by ChatGPT with your Codex account on \(host.name). Audio goes from this phone to ChatGPT."
        }
        return status.message ?? "Codex dictation isn't available on \(host.name)."
    }

    // MARK: Dictation

    private var generation: UInt64 = 0
    /// Waiting on the microphone permission.
    private var starting = false
    private weak var owner: AnyObject?
    private var completion: ((Outcome) -> Void)?
    private var recorder: AVAudioRecorder?
    private var file: URL?
    private var recordingHost: String?
    private var audioActive = false
    private var restoreAudio: (category: AVAudioSession.Category, mode: AVAudioSession.Mode, options: AVAudioSession.CategoryOptions)?

    private struct RecorderUnavailable: Error {}

    var busy: Bool { starting || state != .idle }

    func isOwned(by object: AnyObject) -> Bool { busy && owner === object }

    /// Records for `owner` (a composer) until `stop()`, `cancel()` or the
    /// one-minute cap. `completion` runs once with the outcome, never after
    /// a cancel.
    func start(owner: AnyObject, completion: @escaping (Outcome) -> Void) {
        guard !busy else { return }
        guard let app, !app.voice.live else {
            return completion(.failed("End the voice call to use Codex dictation."))
        }
        guard let host, let status = currentStatus, status.available else {
            return completion(.failed(currentStatus?.message ?? "Codex dictation isn't available right now."))
        }
        generation &+= 1
        let generation = self.generation
        let id = host.id
        self.owner = owner
        self.completion = completion
        starting = true
        changed()
        Task { [weak self] in
            let granted = await AVAudioApplication.requestRecordPermission()
            guard let self, self.generation == generation else { return }
            self.starting = false
            guard granted else { return self.finish(.microphoneDenied) }
            // The prompt can outlast a call starting or the app leaving.
            guard UIApplication.shared.applicationState != .background, self.app?.voice.live == false else { return self.finish(nil) }
            self.record(host: id)
        }
    }

    private func record(host: String) {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("codex-dictation-\(UUID().uuidString).m4a")
        // AAC in an MPEG-4 container (what ChatGPT accepts), at speech quality.
        let settings: [String: Any] = [
            AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
            AVEncoderAudioQualityKey: AVAudioQuality.high.rawValue,
        ]
        let session = AVAudioSession.sharedInstance()
        do {
            restoreAudio = (category: session.category, mode: session.mode, options: session.categoryOptions)
            try session.setCategory(.record, mode: .default, options: [])
            try session.setActive(true)
            audioActive = true
            file = url
            let recorder = try AVAudioRecorder(url: url, settings: settings)
            recorder.delegate = self
            guard recorder.record(forDuration: Self.maxDuration) else { throw RecorderUnavailable() }
            self.recorder = recorder
        } catch {
            return finish(.failed("Couldn’t start recording. Your draft is safe."))
        }
        recordingHost = host
        state = .recording(since: Date())
        changed()
        // Fetches the credential while the user talks.
        if let client = app?.client {
            Task { await client.prepareCodexDictation(hostDeviceId: host) }
        }
    }

    /// Stops recording and transcribes what was said.
    func stop() {
        guard case let .recording(since) = state, let recorder else { return }
        recorder.delegate = nil
        recorder.stop()
        self.recorder = nil
        // A tap-tap holds no words worth an upload.
        guard Date().timeIntervalSince(since) >= 0.5 else { return finish(.noSpeech) }
        transcribe()
    }

    /// Drops the recording, or the result of a transcription in flight. The
    /// draft is untouched.
    func cancel() {
        guard busy else { return }
        generation &+= 1
        finish(nil)
    }

    private func transcribe() {
        // The clip lives on disk only until it is read.
        let audio = file.flatMap { try? Data(contentsOf: $0) }
        discardFile()
        deactivateAudio()
        guard let host = recordingHost, let client = app?.client else {
            return finish(.failed("Couldn’t reach the host device. Your draft is safe."))
        }
        guard let audio, !audio.isEmpty else {
            return finish(.failed("Couldn’t read the recording. Your draft is safe."))
        }
        state = .transcribing
        changed()
        let generation = self.generation
        Task { [weak self] in
            let outcome: Outcome
            do {
                let text = try await client.transcribeCodexDictation(hostDeviceId: host, audio: audio, format: .m4a)
                let words = text.trimmingCharacters(in: .whitespacesAndNewlines)
                outcome = words.isEmpty ? .noSpeech : .text(words)
            } catch CodexDictationError.Failed(let reason) {
                outcome = .failed(reason)
            } catch {
                outcome = .failed("ChatGPT couldn’t transcribe this recording. Your draft is safe.")
            }
            guard let self, self.generation == generation else { return }
            self.finish(outcome)
        }
    }

    private func finish(_ outcome: Outcome?) {
        if let recorder {
            recorder.delegate = nil
            recorder.stop()
            self.recorder = nil
        }
        discardFile()
        deactivateAudio()
        recordingHost = nil
        starting = false
        state = .idle
        let completion = self.completion
        self.completion = nil
        owner = nil
        changed()
        if let outcome { completion?(outcome) }
    }

    private func discardFile() {
        guard let file else { return }
        try? FileManager.default.removeItem(at: file)
        self.file = nil
    }

    /// Other apps' audio resumes; the app's own category comes back.
    private func deactivateAudio() {
        let session = AVAudioSession.sharedInstance()
        if audioActive { try? session.setActive(false, options: .notifyOthersOnDeactivation) }
        audioActive = false
        if let restore = restoreAudio { try? session.setCategory(restore.category, mode: restore.mode, options: restore.options) }
        restoreAudio = nil
    }

    fileprivate func recorderFinished(_ id: ObjectIdentifier, successfully: Bool) {
        guard let recorder, ObjectIdentifier(recorder) == id else { return }
        self.recorder = nil
        // The one-minute cap: transcribe what was said.
        if successfully {
            transcribe()
        } else {
            finish(.failed("Recording stopped unexpectedly. Your draft is safe."))
        }
    }

    // MARK: Observation

    func observe(_ callback: @escaping () -> Void) -> AnyObject {
        let id = UUID()
        observers[id] = callback
        return Token { [weak self] in self?.observers[id] = nil }
    }

    private func changed() {
        for callback in observers.values { callback() }
    }

    private final class Token {
        let cancel: () -> Void
        init(cancel: @escaping () -> Void) { self.cancel = cancel }
        deinit { cancel() }
    }
}

extension CodexDictationController: AVAudioRecorderDelegate {
    nonisolated func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
        let id = ObjectIdentifier(recorder)
        Task { @MainActor [weak self] in self?.recorderFinished(id, successfully: flag) }
    }

    nonisolated func audioRecorderEncodeErrorDidOccur(_ recorder: AVAudioRecorder, error: Error?) {
        let id = ObjectIdentifier(recorder)
        Task { @MainActor [weak self] in self?.recorderFinished(id, successfully: false) }
    }
}
