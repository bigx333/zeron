//! Window-owned voice orchestrator. Each session runs in the host's own
//! projectless Codex chat, resumed across calls, and survives navigation
//! between threads; only an explicit end, an engine change or a provider
//! failure closes it. No automatic reconnect or microphone resume.
use crate::orb::OrbState;
use crate::state::EngineHandle;
#[cfg(target_os = "macos")]
use futures::FutureExt as _;
use futures::StreamExt as _;
use gpui::{Context, Task};
use gpui_tokio::Tokio;
use zeron_proto::voice::*;
mod permissions;
mod remote;
pub(crate) mod shortcut;

pub struct VoiceController {
    pub phase: VoicePhase,
    pub chat_id: Option<String>,
    pub reason: Option<VoiceRejection>,
    pub snapshot: Option<VoiceSnapshot>,
    /// The newest speaker turn's live text, then its final.
    pub caption: zeron_voice_session::Caption,
    pub microphone_level: u16,
    pub speaker_level: u16,
    /// The full-window stage is presented over the shell.
    pub stage_open: bool,
    pub host_name: Option<String>,
    speaker_last_loud: Option<std::time::Instant>,
    /// When the session first became active; drives the stage's call timer.
    pub active_since: Option<std::time::Instant>,
    engine: Option<EngineHandle>,
    controls: Option<tokio::sync::watch::Sender<bool>>,
    push_to_talk: bool,
    manually_muted: bool,
    shortcut_held: bool,
    button_held: bool,
    shortcut_combo: String,
    shortcut: Option<shortcut::Registration>,
    shortcut_task: Option<Task<()>>,
    _input_settings: Option<gpui::Subscription>,
    _shortcut_recording: Option<Task<()>>,
    pub push_to_talk_error: Option<String>,
    epoch: u64,
    task: Option<Task<()>>,
    cancellation: tokio_util::sync::CancellationToken,
    /// Whether this device's Codex is installed and enabled: no Codex, no
    /// voice. `None` until the catalog answers.
    codex_ready: Option<bool>,
    /// The connection `codex_ready` was read from, and its read in flight.
    codex_engine: Option<EngineHandle>,
    codex_check: Option<Task<()>>,
    /// This call played its start sound and owes its end sound.
    cued: bool,
}
impl Default for VoiceController {
    fn default() -> Self {
        Self {
            phase: VoicePhase::Closed,
            chat_id: None,
            reason: None,
            snapshot: None,
            caption: Default::default(),
            microphone_level: 0,
            speaker_level: 0,
            stage_open: false,
            host_name: None,
            speaker_last_loud: None,
            active_since: None,
            engine: None,
            controls: None,
            push_to_talk: false,
            manually_muted: false,
            shortcut_held: false,
            button_held: false,
            shortcut_combo: crate::settings::ShortcutId::PushToTalk
                .default_combo()
                .into(),
            shortcut: None,
            shortcut_task: None,
            _input_settings: None,
            _shortcut_recording: None,
            push_to_talk_error: None,
            epoch: 0,
            task: None,
            cancellation: tokio_util::sync::CancellationToken::new(),
            codex_ready: None,
            codex_engine: None,
            codex_check: None,
            cued: false,
        }
    }
}
impl VoiceController {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let input_settings = cx.observe_global::<crate::settings::SettingsStore>(|this, cx| {
            this.sync_input_settings(cx);
        });
        let mut recording = shortcut::subscribe_recording();
        let shortcut_recording = cx.spawn(async move |this, cx| {
            while recording.next().await.is_some() {
                if this
                    .update(cx, |this, cx| this.refresh_shortcut(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let settings = crate::settings::current(cx);
        let mut controller = Self::default();
        controller.push_to_talk = settings.voice_push_to_talk;
        controller.shortcut_combo = settings.keymap.push_to_talk;
        controller._input_settings = Some(input_settings);
        controller._shortcut_recording = Some(shortcut_recording);
        controller
    }

    fn sync_input_settings(&mut self, cx: &mut Context<Self>) {
        let settings = crate::settings::current(cx);
        if self.push_to_talk == settings.voice_push_to_talk
            && self.shortcut_combo == settings.keymap.push_to_talk
        {
            return;
        }
        if self.push_to_talk != settings.voice_push_to_talk {
            self.manually_muted = false;
        }
        self.push_to_talk = settings.voice_push_to_talk;
        self.shortcut_combo = settings.keymap.push_to_talk;
        self.refresh_shortcut(cx);
    }

    fn refresh_shortcut(&mut self, cx: &mut Context<Self>) {
        self.shortcut_task = None;
        self.shortcut = None;
        self.shortcut_held = false;
        self.button_held = false;
        self.push_to_talk_error = None;
        self.apply_mute(cx);
        if !self.is_live() || !self.push_to_talk || shortcut::is_recording() {
            return;
        }
        let registered = shortcut::Shortcut::parse(&self.shortcut_combo)
            .map_err(|_| "Choose a valid global shortcut in Settings → Voice.".to_owned())
            .and_then(shortcut::register);
        let (registration, mut events) = match registered {
            Ok(registered) => registered,
            Err(error) => {
                self.push_to_talk_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.shortcut = Some(registration);
        self.shortcut_task = Some(cx.spawn(async move |this, cx| {
            #[cfg(target_os = "macos")]
            let mut held = false;
            loop {
                #[cfg(target_os = "macos")]
                let event = if held {
                    futures::select_biased! {
                        event = events.next().fuse() => event,
                        _ = cx.background_executor().timer(std::time::Duration::from_millis(20)).fuse() => {
                            match this.update(cx, |this, _| this.shortcut.as_ref().is_some_and(|key| key.modifiers_held())) {
                                Ok(true) => continue,
                                Ok(false) => Some(shortcut::Event::Held(false)),
                                Err(_) => break,
                            }
                        },
                    }
                } else {
                    events.next().await
                };
                #[cfg(not(target_os = "macos"))]
                let event = events.next().await;
                let disconnected = event.is_none();
                let event = event.unwrap_or_else(|| shortcut::Event::Unavailable(
                    "Global push-to-talk disconnected. End the call and try again.".into()
                ));
                #[cfg(target_os = "macos")]
                { held = matches!(event, shortcut::Event::Held(true)); }
                if this.update(cx, |this, cx| match event {
                    shortcut::Event::Held(held) => {
                        #[cfg(target_os = "macos")]
                        let held = held && this.shortcut.as_ref().is_some_and(|key| key.modifiers_held());
                        this.set_shortcut_held(held, cx);
                    },
                    shortcut::Event::Unavailable(error) => {
                        this.push_to_talk_error = Some(error);
                        this.set_shortcut_held(false, cx);
                        cx.notify();
                    }
                }).is_err() || disconnected {
                    break;
                }
            }
        }));
    }

    fn set_shortcut_held(&mut self, held: bool, cx: &mut Context<Self>) {
        let held = held && self.is_live() && self.push_to_talk && !shortcut::is_recording();
        if self.shortcut_held != held {
            self.shortcut_held = held;
            self.apply_mute(cx);
        }
    }

    pub fn push_to_talk(&self) -> bool {
        self.push_to_talk
    }

    pub fn hold_button(&mut self, held: bool, cx: &mut Context<Self>) {
        let held = held && self.is_live() && self.push_to_talk && !shortcut::is_recording();
        if self.button_held != held {
            self.button_held = held;
            self.apply_mute(cx);
        }
    }

    fn apply_mute(&mut self, cx: &mut Context<Self>) {
        let muted = self.muted();
        if let Some(snapshot) = &mut self.snapshot {
            snapshot.muted = muted;
        }
        if let Some(controls) = &self.controls {
            controls.send_replace(muted);
        }
        cx.notify();
    }
    /// Voice can be offered here. An unread catalog counts as yes, so a slow
    /// or failed read never hides it; a live call or its failure stays.
    pub fn offered(&self) -> bool {
        self.codex_ready != Some(false) || self.is_live() || self.reason.is_some()
    }

    /// Read whether `engine`'s device can take a call: installed and enabled
    /// in Settings → Agents. `force` re-reads a connection already read.
    pub fn check_codex(&mut self, engine: &EngineHandle, force: bool, cx: &mut Context<Self>) {
        let same = self
            .codex_engine
            .as_ref()
            .is_some_and(|e| e.same_connection(engine));
        if same && !force {
            return;
        }
        if !same {
            self.codex_ready = None;
        }
        self.codex_engine = Some(engine.clone());
        let engine = engine.clone();
        self.codex_check = Some(cx.spawn(async move |this, cx| {
            let ready = engine
                .client()
                .call(zeron_rpc::methods::LIST_HARNESSES, serde_json::json!({}))
                .await
                .ok()
                .and_then(|value| {
                    serde_json::from_value::<Vec<zeron_engine::registry::HarnessDescriptor>>(value)
                        .ok()
                })
                .map(|list| {
                    list.iter().any(|d| {
                        d.id == zeron_proto::HarnessId::Codex
                            && d.installed
                            && zeron_engine::registry::descriptor_enabled(d)
                    })
                });
            this.update(cx, |voice, cx| {
                if ready.is_some() && ready != voice.codex_ready {
                    voice.codex_ready = ready;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// A session is underway (including its preparation and native stop).
    pub fn is_live(&self) -> bool {
        matches!(self.phase, VoicePhase::Checking) || self.phase.replaces_composer()
    }

    /// Start an orchestrator in a new projectless Codex chat on `host_device_id`.
    /// `config` is the chat's Codex configuration; its harness must be Codex.
    pub fn start(
        &mut self,
        engine: EngineHandle,
        host_device_id: String,
        config: zeron_proto::ChatConfig,
        voice: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.cancel(cx);
        self.sync_input_settings(cx);
        // The call sounds as it is placed, well before the microphone opens.
        crate::sound::play_voice(true);
        self.cued = true;
        self.engine = Some(engine.clone());
        self.phase = VoicePhase::Checking;
        self.chat_id = None;
        self.reason = None;
        let epoch = self.epoch;
        let cancellation = self.cancellation.clone();
        let (controls, muted) = tokio::sync::watch::channel(self.muted());
        self.controls = Some(controls);
        self.refresh_shortcut(cx);
        let (events, mut event_rx) = tokio::sync::mpsc::channel(32);
        let query = Tokio::spawn(cx, async move {
            // Media belongs to this viewport, even when the chosen host is
            // this device. The execution host never opens audio devices.
            remote::run(
                engine,
                host_device_id,
                config,
                voice,
                cancellation,
                events,
                muted,
            )
            .await
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let receive = async {
                while let Some(event) = event_rx.recv().await {
                    if this
                        .update(cx, |controller, cx| {
                            if controller.epoch == epoch {
                                controller.reduce(event, cx);
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            };
            receive.await;
            let result = query.await.unwrap_or(Err(VoiceRejection::Protocol));
            let _ = this.update(cx, |controller, cx| {
                if controller.epoch != epoch {
                    return;
                }
                match result {
                    // A provider failure already reduced to Failed keeps its reason.
                    Ok(()) if controller.phase == VoicePhase::Failed => {}
                    Ok(()) => {
                        controller.phase = VoicePhase::Closed;
                    }
                    Err(reason) => {
                        controller.phase = VoicePhase::Failed;
                        controller.reason = Some(reason);
                    }
                }
                controller.reset_session();
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Ending now sounds: a hang-up or a connected call ending. A failure to
    /// connect only explains itself.
    fn end_sounds(&self) -> bool {
        self.cued && (self.active_since.is_some() || self.reason.is_none())
    }

    fn reset_session(&mut self) {
        if self.end_sounds() {
            crate::sound::play_voice(false);
        }
        self.cued = false;
        self.snapshot = None;
        self.caption.clear();
        self.microphone_level = 0;
        self.speaker_level = 0;
        if let Some(controls) = self.controls.take() {
            controls.send_replace(true);
        }
        self.shortcut_task = None;
        self.shortcut = None;
        self.shortcut_held = false;
        self.button_held = false;
        self.manually_muted = false;
        self.push_to_talk_error = None;
        self.task = None;
        self.stage_open = false;
        self.active_since = None;
    }

    /// End the session (any phase). The provider stop runs in the background.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.epoch = self.epoch.wrapping_add(1);
        self.cancellation.cancel();
        self.cancellation = tokio_util::sync::CancellationToken::new();
        self.reset_session();
        self.phase = VoicePhase::Closed;
        self.chat_id = None;
        self.reason = None;
        self.engine = None;
        cx.notify();
    }

    pub fn set_stage_open(&mut self, open: bool, cx: &mut Context<Self>) {
        let open = open && self.is_live();
        if !open {
            self.hold_button(false, cx);
        }
        if self.stage_open != open {
            self.stage_open = open;
            cx.notify();
        }
    }

    /// Forget a failure once it has been seen.
    pub fn dismiss_reason(&mut self, cx: &mut Context<Self>) {
        if self.reason.take().is_some() {
            if self.phase == VoicePhase::Failed {
                self.phase = VoicePhase::Closed;
            }
            cx.notify();
        }
    }

    pub fn belongs_to(&self, engine: &EngineHandle) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|e| e.same_connection(engine))
    }
    pub fn muted(&self) -> bool {
        self.manually_muted || (self.push_to_talk && !(self.shortcut_held || self.button_held))
    }
    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        if !self.push_to_talk {
            self.manually_muted = !self.manually_muted;
            self.apply_mute(cx);
        }
    }

    /// The utterance the caption shows (a speaker turn), if the provider names it.
    pub fn caption_item(&self) -> Option<&str> {
        self.caption.item()
    }

    pub fn orb_state(&self) -> OrbState {
        orb_state(self.phase, self.snapshot.as_ref())
    }

    /// Normalized 0…1 microphone loudness; zero while muted.
    pub fn microphone_level(&self) -> f32 {
        if self.muted() {
            return 0.0;
        }
        f32::from(self.microphone_level) / f32::from(u16::MAX)
    }

    /// Normalized 0…1 speaker peak; the orb smooths each channel separately.
    pub fn speaker_level(&self) -> f32 {
        f32::from(self.speaker_level) / f32::from(u16::MAX)
    }

    pub fn reduce(&mut self, event: VoiceEvent, cx: &mut Context<Self>) {
        match event {
            VoiceEvent::Snapshot { mut snapshot } => {
                snapshot.muted = self.muted();
                if let Some(previous) = &self.snapshot {
                    snapshot.playing = previous.playing;
                }
                if self.chat_id.is_none() {
                    self.chat_id = Some(snapshot.chat_id.clone());
                }

                if self.chat_id.as_deref() != Some(snapshot.chat_id.as_str()) {
                    return;
                }
                if self.snapshot.as_ref().is_some_and(|old| {
                    old.generation != snapshot.generation || old.session_id != snapshot.session_id
                }) {
                    return;
                }
                self.phase = snapshot.phase;
                if self.phase == VoicePhase::Active && self.active_since.is_none() {
                    self.active_since = Some(std::time::Instant::now());
                }
                self.snapshot = Some(snapshot);
            }
            VoiceEvent::Partial {
                generation,
                item_id,
                text,
            } if self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.generation == generation) =>
            {
                if !self.caption.partial(item_id, &text) {
                    self.cancel(cx);
                    self.phase = VoicePhase::Failed;
                    self.reason = Some(VoiceRejection::Overflow);
                }
            }
            VoiceEvent::Levels {
                generation,
                microphone,
                speaker,
            } if self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.generation == generation) =>
            {
                self.microphone_level = microphone;
                self.speaker_level = speaker;
                if let Some(snapshot) = &mut self.snapshot {
                    let threshold = if snapshot.playing { 328 } else { 655 };
                    if speaker >= threshold {
                        snapshot.playing = true;
                        self.speaker_last_loud = Some(std::time::Instant::now());
                    } else if self
                        .speaker_last_loud
                        .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(250))
                    {
                        snapshot.playing = false;
                    }
                }
            }
            // The final segment keeps showing as the caption until the next
            // item starts; the canonical copy is already in the transcript.
            VoiceEvent::Final { transcript }
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.session_id == transcript.session_id) =>
            {
                self.caption.complete(&transcript);
            }
            VoiceEvent::Closed { generation, reason }
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.generation == generation) =>
            {
                self.phase = if reason.is_some() {
                    VoicePhase::Failed
                } else {
                    VoicePhase::Closed
                };
                self.reason = reason;
                self.reset_session();
            }
            _ => return,
        }
        cx.notify();
    }
    pub fn reason_text(&self) -> &'static str {
        match self.reason {
            Some(VoiceRejection::CreditExclusionUnverified) => {
                "Codex controls subscription usage and any enabled additional credits."
            }
            Some(VoiceRejection::WrongHarness) => "Voice requires Codex.",
            Some(VoiceRejection::RemoteHost) => "Voice runs on this device only.",
            Some(VoiceRejection::IncludedUsageUnavailable) => {
                "Codex usage is currently unavailable."
            }
            Some(VoiceRejection::AudioFormatUnverified) => {
                "This Codex voice format has not been verified."
            }
            Some(VoiceRejection::ChatgptRequired) => "Sign in to Codex with ChatGPT to use voice.",
            // Remote calls still play audio here, through this device's Codex helper.
            Some(VoiceRejection::NativeRuntimeUnavailable) => {
                "Install or update Codex on this device with its official installer to use voice. npm installs lack the voice runtime."
            }
            Some(VoiceRejection::DeviceUnavailable) => {
                "Check microphone permission and your audio devices."
            }
            Some(VoiceRejection::MicrophonePermissionDenied) => {
                "Allow Zeron Nova to use the microphone in System Settings → Privacy & Security → Microphone."
            }
            Some(VoiceRejection::MicrophoneMetadataMissing) => {
                "Microphone setup is missing. Rebuild or reinstall Zeron Nova, then restart it."
            }
            Some(VoiceRejection::Busy) => "Another window already owns the voice session.",
            Some(_) => "Voice could not connect. Try again in a moment.",
            None => "",
        }
    }
}

/// Call-timer text: `m:ss`, or `h:mm:ss` past the hour.
pub fn format_elapsed(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Shared with the mobile apps so every orb reads the call the same way.
pub use zeron_voice_session::orb_state;

impl Drop for VoiceController {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    fn snapshot() -> VoiceSnapshot {
        VoiceSnapshot {
            session_id: "one".into(),
            chat_id: "chat".into(),
            generation: 2,
            phase: VoicePhase::Active,
            muted: false,
            playing: false,
            work: VoiceWork::Idle,
            reason: None,
            voice: None,
            voices: Vec::new(),
        }
    }

    #[gpui::test]
    fn push_to_talk_starts_muted_and_only_transmits_while_a_control_is_held(
        cx: &mut gpui::TestAppContext,
    ) {
        let voice = cx.new(|_| {
            let mut voice = VoiceController::default();
            voice.phase = VoicePhase::Checking;
            voice.push_to_talk = true;
            voice
        });
        voice.update(cx, |voice, cx| {
            let (controls, mut muted) = tokio::sync::watch::channel(voice.muted());
            voice.controls = Some(controls);
            assert!(
                *muted.borrow_and_update(),
                "startup must never briefly open the microphone"
            );
            voice.reduce(
                VoiceEvent::Snapshot {
                    snapshot: snapshot(),
                },
                cx,
            );
            assert!(
                voice.snapshot.as_ref().unwrap().muted,
                "host snapshots cannot reopen the local microphone"
            );
            voice.toggle_mute(cx);
            assert!(voice.muted(), "clicks and M cannot latch push-to-talk open");

            voice.set_shortcut_held(true, cx);
            assert!(!*muted.borrow_and_update());
            voice.reduce(
                VoiceEvent::Snapshot {
                    snapshot: snapshot(),
                },
                cx,
            );
            assert!(!voice.snapshot.as_ref().unwrap().muted);
            voice.set_shortcut_held(true, cx); // key repeat
            assert!(!muted.has_changed().unwrap());
            voice.set_shortcut_held(false, cx);
            assert!(*muted.borrow_and_update());
            assert_eq!(voice.microphone_level(), 0.0);

            voice.set_shortcut_held(true, cx);
            voice.set_shortcut_held(false, cx);
            assert!(
                *muted.borrow_and_update(),
                "rapid press/release keeps only the latest state"
            );

            voice.hold_button(true, cx);
            voice.set_shortcut_held(true, cx);
            voice.hold_button(false, cx);
            assert!(
                !*muted.borrow_and_update(),
                "the shortcut is still physically held"
            );
            voice.set_shortcut_held(false, cx);
            assert!(*muted.borrow_and_update());
        });
    }

    #[gpui::test]
    fn push_to_talk_hiding_the_hold_button_and_ending_a_call_close_the_microphone(
        cx: &mut gpui::TestAppContext,
    ) {
        let voice = cx.new(|_| {
            let mut voice = VoiceController::default();
            voice.phase = VoicePhase::Active;
            voice.push_to_talk = true;
            voice.stage_open = true;
            voice
        });
        voice.update(cx, |voice, cx| {
            let (controls, mut muted) = tokio::sync::watch::channel(true);
            voice.controls = Some(controls);
            voice.hold_button(true, cx);
            assert!(!*muted.borrow_and_update());
            voice.set_stage_open(false, cx);
            assert!(*muted.borrow_and_update());
            voice.set_shortcut_held(true, cx);
            assert!(!*muted.borrow_and_update());
            voice.cancel(cx);
            assert!(*muted.borrow_and_update());
            voice.set_shortcut_held(true, cx);
            voice.hold_button(true, cx);
            assert!(voice.muted(), "late presses cannot reopen a closed call");
        });
    }

    #[gpui::test]
    fn push_to_talk_setting_changes_clear_holds_and_preserve_hands_free_muting(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            crate::settings::init(crate::settings::UiSettings::default(), dir.path(), cx)
        });
        let voice = cx.new(|_| VoiceController::default());
        voice.update(cx, |voice, cx| {
            voice.phase = VoicePhase::Active;
            voice.toggle_mute(cx);
            assert!(voice.muted());
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |s| {
                s.keymap.push_to_talk = "ctrl-alt-v".into();
            });
            voice.sync_input_settings(cx);
            assert!(
                voice.muted(),
                "editing the hold shortcut preserves a hands-free manual mute"
            );
            voice.push_to_talk = true;
            voice.manually_muted = false;
            voice.shortcut_held = true;
            assert!(!voice.muted());
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |s| {
                s.voice_push_to_talk = true;
                s.keymap.push_to_talk = "unsupported-key".into();
            });
            voice.sync_input_settings(cx);
            assert!(voice.muted(), "rebinding requires a fresh press");
            assert!(
                voice.push_to_talk_error.is_some(),
                "registration failures explain why the mic stays closed"
            );
            crate::settings::update(crate::settings::SavePolicy::Immediate, cx, |s| {
                s.voice_push_to_talk = false
            });
            voice.sync_input_settings(cx);
            assert!(
                !voice.muted(),
                "switching back to hands-free restores its normal behavior"
            );
            voice.toggle_mute(cx);
            assert!(voice.muted());
        });
    }

    #[test]
    fn voice_is_offered_unless_codex_is_known_missing() {
        let mut voice = VoiceController::default();
        assert!(voice.offered(), "an unread catalog never hides voice");
        voice.codex_ready = Some(true);
        assert!(voice.offered());
        voice.codex_ready = Some(false);
        assert!(!voice.offered());
        // A failure explains itself before the trigger goes away.
        voice.reason = Some(VoiceRejection::NativeRuntimeUnavailable);
        assert!(voice.offered());
        voice.reason = None;
        voice.phase = VoicePhase::Checking;
        assert!(voice.offered(), "a call in progress keeps its trigger");
    }

    #[test]
    fn only_a_hang_up_or_a_connected_call_sounds_its_end() {
        let mut voice = VoiceController::default();
        assert!(!voice.end_sounds(), "no call was placed");
        voice.cued = true;
        assert!(voice.end_sounds(), "hanging up while connecting");
        voice.reason = Some(VoiceRejection::Busy);
        assert!(!voice.end_sounds(), "a failure to connect stays quiet");
        voice.active_since = Some(std::time::Instant::now());
        assert!(voice.end_sounds(), "a connected call that drops");
    }

    #[gpui::test]
    fn voice_reducer_ignores_stale_and_foreign_events(cx: &mut gpui::TestAppContext) {
        let voice = cx.new(|_| VoiceController::default());
        voice.update(cx, |voice, cx| {
            voice.chat_id = Some("chat".into());
            let snapshot = snapshot();
            voice.reduce(
                VoiceEvent::Snapshot {
                    snapshot: snapshot.clone(),
                },
                cx,
            );
            voice.reduce(
                VoiceEvent::Partial {
                    generation: 1,
                    item_id: None,
                    text: "old".into(),
                },
                cx,
            );
            assert!(voice.caption.text().is_empty());
            voice.reduce(
                VoiceEvent::Partial {
                    generation: 2,
                    item_id: None,
                    text: "current".into(),
                },
                cx,
            );
            let mut foreign = snapshot;
            foreign.chat_id = "other".into();
            foreign.generation = 3;
            voice.reduce(VoiceEvent::Snapshot { snapshot: foreign }, cx);
            assert_eq!(voice.snapshot.as_ref().unwrap().generation, 2);
            voice.reduce(
                VoiceEvent::Closed {
                    generation: 1,
                    reason: None,
                },
                cx,
            );
            assert!(voice.is_live());
            voice.set_stage_open(true, cx);
            assert!(voice.stage_open);
            voice.cancel(cx);
            assert!(!voice.is_live());
            assert!(!voice.stage_open);
            assert!(voice.caption.text().is_empty());
            // A closed session never reopens the stage.
            voice.set_stage_open(true, cx);
            assert!(!voice.stage_open);
        });
    }

    /// Codex delivers the user's final after the answer started streaming:
    /// the caption stays on the answer and keeps every streamed word.
    #[gpui::test]
    fn late_user_final_keeps_the_streaming_answer_captioned(cx: &mut gpui::TestAppContext) {
        let voice = cx.new(|_| VoiceController::default());
        voice.update(cx, |voice, cx| {
            voice.chat_id = Some("chat".into());
            voice.reduce(
                VoiceEvent::Snapshot {
                    snapshot: snapshot(),
                },
                cx,
            );
            let partial = |item: &str, text: &str| VoiceEvent::Partial {
                generation: 2,
                item_id: Some(item.into()),
                text: text.into(),
            };
            voice.reduce(partial("user", "Open the test"), cx);
            voice.reduce(partial("answer", "Sure, "), cx);
            voice.reduce(
                VoiceEvent::Final {
                    transcript: VoiceTranscript {
                        session_id: "one".into(),
                        item_id: "user".into(),
                        role: VoiceRole::User,
                        text: "Open the test.".into(),
                        promoted_message_id: None,
                    },
                },
                cx,
            );
            voice.reduce(partial("answer", "opening it."), cx);
            assert_eq!(voice.caption.text(), "Sure, opening it.");
            assert_eq!(voice.caption_item(), Some("answer"));
        });
    }

    #[gpui::test]
    fn provider_failure_closes_the_stage_and_keeps_its_reason(cx: &mut gpui::TestAppContext) {
        let voice = cx.new(|_| VoiceController::default());
        voice.update(cx, |voice, cx| {
            voice.chat_id = Some("chat".into());
            voice.reduce(
                VoiceEvent::Snapshot {
                    snapshot: snapshot(),
                },
                cx,
            );
            voice.set_stage_open(true, cx);
            voice.reduce(
                VoiceEvent::Closed {
                    generation: 2,
                    reason: Some(VoiceRejection::DeviceUnavailable),
                },
                cx,
            );
            assert_eq!(voice.phase, VoicePhase::Failed);
            assert!(!voice.stage_open);
            assert!(!voice.reason_text().is_empty());
            voice.dismiss_reason(cx);
            assert_eq!(voice.phase, VoicePhase::Closed);
        });
    }

    #[test]
    fn call_timer_reads_like_a_call() {
        assert_eq!(format_elapsed(0), "0:00");
        assert_eq!(format_elapsed(75), "1:15");
        assert_eq!(format_elapsed(3600 + 62), "1:01:02");
    }

    #[test]
    fn visual_state_keeps_muting_and_playback_orthogonal() {
        let mut s = snapshot();
        s.muted = true;
        s.playing = true;
        assert_eq!(orb_state(s.phase, Some(&s)), OrbState::Composing);
        s.playing = false;
        assert_eq!(orb_state(s.phase, Some(&s)), OrbState::Breathing);
        s.muted = false;
        assert_eq!(orb_state(s.phase, Some(&s)), OrbState::Listening);
        s.work = VoiceWork::Working;
        assert_eq!(orb_state(s.phase, Some(&s)), OrbState::Working);
        s.work = VoiceWork::AwaitingInput;
        assert_eq!(orb_state(s.phase, Some(&s)), OrbState::Solving);
        assert_eq!(
            orb_state(VoicePhase::Starting, Some(&s)),
            OrbState::Connecting
        );
    }
}
