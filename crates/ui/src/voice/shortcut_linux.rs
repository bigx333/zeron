use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _, channel::mpsc};
use tokio_util::sync::CancellationToken;

use super::{Event, Registration, Shortcut};

pub(in crate::voice) fn register(
    shortcut: Shortcut,
) -> Result<(Registration, mpsc::UnboundedReceiver<Event>), String> {
    let (tx, rx) = mpsc::unbounded();
    let cancel = CancellationToken::new();
    let stopped = cancel.clone();
    std::thread::Builder::new()
        .name("voice-push-to-talk".into())
        .spawn(move || {
            // A replacement waits for the cancelled registration to release
            // its grab/session before attempting to bind the same chord.
            static OWNER: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _owner = OWNER.lock().unwrap();
            if stopped.is_cancelled() { return; }
            let result = if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
                futures::executor::block_on(portal(&shortcut, &tx, stopped))
            } else {
                x11(&shortcut, &tx, stopped)
            };
            let _ = tx.unbounded_send(Event::Held(false));
            if let Err(error) = result {
                let _ = tx.unbounded_send(Event::Unavailable(format!(
                    "Global push-to-talk unavailable: {error}. Choose another shortcut in Settings → Voice."
                )));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok((Registration(cancel), rx))
}

fn x11(
    shortcut: &Shortcut,
    tx: &mpsc::UnboundedSender<Event>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event as XEvent,
            xproto::{ConnectionExt as _, ModMask},
        },
    };
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let key = crate::appshots::register_x11_shortcut(&connection, root, shortcut)?;
    connection.flush()?;
    let required = (if shortcut.control { 4 } else { 0 })
        | (if shortcut.alt { 8 } else { 0 })
        | (if shortcut.shift { 1 } else { 0 })
        | (if shortcut.platform { 64 } else { 0 });
    let mut held = false;
    let mut latched = false;
    while !cancel.is_cancelled() && !tx.is_closed() {
        let mut pressed = false;
        while let Some(event) = connection.poll_for_event()? {
            if let XEvent::KeyPress(event) = event {
                pressed |= event.detail == key;
            }
        }
        if !pressed && !held && !latched {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let keys = connection.query_keymap()?.reply()?.keys;
        let key_down = keys[usize::from(key / 8)] & (1 << (key % 8)) != 0;
        let modifiers = u16::from(connection.query_pointer(root)?.reply()?.mask);
        let chord_down = key_down && modifiers & required == required;
        // Query physical state instead of treating X11's repeat release/press
        // pairs as real releases. Releasing a modifier also closes the mic.
        let next = chord_down && (held || (pressed && !latched));
        if pressed && key_down {
            latched = true;
        }
        if !key_down {
            latched = false;
        }
        if next != held {
            held = next;
            if tx.unbounded_send(Event::Held(held)).is_err() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    connection.ungrab_key(key, root, ModMask::ANY)?.check()?;
    connection.flush()?;
    Ok(())
}

async fn portal(
    shortcut: &Shortcut,
    tx: &mpsc::UnboundedSender<Event>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    use ashpd::desktop::{
        CreateSessionOptions,
        global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut},
    };
    const ID: &str = "voice-push-to-talk";
    let portal = GlobalShortcuts::new().await?;
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await?;
    let session_path = serde_json::to_value(&session)?
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("invalid shortcut session"))?
        .to_owned();
    // Closing the portal session releases its system registration, including
    // when a bind/consent request is cancelled while still in progress.
    let operation = async {
        let mut presses = portal.receive_activated().await?;
        let mut releases = portal.receive_deactivated().await?;
        let mut closed = session.receive_closed().await?;
        let trigger = shortcut.portal_trigger();
        let binding =
            NewShortcut::new(ID, "Push to talk in Zeron").preferred_trigger(Some(trigger.as_str()));
        portal
            .bind_shortcuts(&session, &[binding], None, BindShortcutsOptions::default())
            .await?
            .response()?;
        let mut latest = None;
        loop {
            futures::select_biased! {
                _ = closed.next().fuse() => return Err(anyhow::anyhow!("desktop shortcut session closed")),
                release = releases.next().fuse() => {
                    let release = release.ok_or_else(|| anyhow::anyhow!("desktop shortcut service disconnected"))?;
                    if release.shortcut_id() == ID && release.session_handle().as_str() == session_path
                        && latest.is_none_or(|(at, _)| release.timestamp() >= at)
                    {
                        latest = Some((release.timestamp(), false));
                        tx.unbounded_send(Event::Held(false))?;
                    }
                },
                press = presses.next().fuse() => {
                    let press = press.ok_or_else(|| anyhow::anyhow!("desktop shortcut service disconnected"))?;
                    // Signals arrive on separate streams. An already queued
                    // release wins over an older press (including ties).
                    if press.shortcut_id() == ID && press.session_handle().as_str() == session_path
                        && latest.is_none_or(|(at, held)| press.timestamp() > at || (press.timestamp() == at && held))
                    {
                        latest = Some((press.timestamp(), true));
                        tx.unbounded_send(Event::Held(true))?;
                    }
                },
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let result = futures::select_biased! {
        _ = cancel.cancelled().fuse() => Ok(()),
        result = operation.fuse() => result,
    };
    let _ = session.close().await;
    result
}
