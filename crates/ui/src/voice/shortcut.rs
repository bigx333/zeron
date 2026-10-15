//! Hold shortcuts are registered only for the lifetime of a push-to-talk call.
//! Recording any shortcut suspends them and closes the microphone first.
use std::sync::{Mutex, OnceLock};

use futures::channel::mpsc;

pub(super) use crate::appshots::shortcut::Shortcut;

#[derive(Debug)]
pub(crate) enum Event {
    Held(bool),
    Unavailable(String),
}

#[derive(Default)]
struct Recording {
    active: bool,
    subscribers: Vec<mpsc::UnboundedSender<bool>>,
}

fn recording() -> &'static Mutex<Recording> {
    static RECORDING: OnceLock<Mutex<Recording>> = OnceLock::new();
    RECORDING.get_or_init(Default::default)
}

pub(crate) fn set_recording(active: bool) {
    let mut recording = recording().lock().unwrap();
    if recording.active == active {
        return;
    }
    recording.active = active;
    recording
        .subscribers
        .retain(|tx| tx.unbounded_send(active).is_ok());
}

pub(super) fn is_recording() -> bool {
    recording().lock().unwrap().active
}

pub(super) fn subscribe_recording() -> mpsc::UnboundedReceiver<bool> {
    let (tx, rx) = mpsc::unbounded();
    let mut recording = recording().lock().unwrap();
    recording.subscribers.push(tx);
    rx
}

#[cfg(target_os = "macos")]
pub(super) type Registration = crate::appshots::HoldShortcut;
#[cfg(target_os = "macos")]
pub(super) fn register(
    shortcut: Shortcut,
) -> Result<(Registration, mpsc::UnboundedReceiver<Event>), String> {
    crate::appshots::register_hold_shortcut(shortcut)
}

#[cfg(target_os = "linux")]
#[path = "shortcut_linux.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "shortcut_windows.rs"]
mod platform;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) use platform::register;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) struct Registration(pub tokio_util::sync::CancellationToken);
#[cfg(any(target_os = "linux", target_os = "windows"))]
impl Drop for Registration {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
