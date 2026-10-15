use std::time::Duration;

use futures::channel::mpsc;
use windows::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};

use super::{Event, Registration, Shortcut};

pub(in crate::voice) fn register(
    shortcut: Shortcut,
) -> Result<(Registration, mpsc::UnboundedReceiver<Event>), String> {
    let key =
        virtual_key(&shortcut.key).ok_or_else(|| "Unsupported global shortcut key.".to_owned())?;
    let (tx, rx) = mpsc::unbounded();
    let cancel = tokio_util::sync::CancellationToken::new();
    let stopped = cancel.clone();
    std::thread::Builder::new()
        .name("voice-push-to-talk".into())
        .spawn(move || unsafe {
            static OWNER: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _owner = OWNER.lock().unwrap();
            if stopped.is_cancelled() { return; }
            let modifiers = MOD_NOREPEAT
                | if shortcut.control { MOD_CONTROL } else { HOT_KEY_MODIFIERS(0) }
                | if shortcut.alt { MOD_ALT } else { HOT_KEY_MODIFIERS(0) }
                | if shortcut.shift { MOD_SHIFT } else { HOT_KEY_MODIFIERS(0) }
                | if shortcut.platform { MOD_WIN } else { HOT_KEY_MODIFIERS(0) };
            const ID: i32 = 0x5a50;
            if let Err(error) = RegisterHotKey(None, ID, modifiers, u32::from(key)) {
                let _ = tx.unbounded_send(Event::Unavailable(format!(
                    "Push-to-talk shortcut unavailable: {error}. Choose another shortcut in Settings → Voice."
                )));
                return;
            }
            let mut held = false;
            while !stopped.is_cancelled() && !tx.is_closed() {
                let mut message = MSG::default();
                let mut pressed = false;
                while PeekMessageW(&mut message, None, WM_HOTKEY, WM_HOTKEY, PM_REMOVE).as_bool() {
                    pressed |= message.wParam.0 == ID as usize;
                }
                let down = |key: u16| GetAsyncKeyState(i32::from(key)) < 0;
                let chord = down(key)
                    && (!shortcut.control || down(VK_CONTROL.0))
                    && (!shortcut.alt || down(VK_MENU.0))
                    && (!shortcut.shift || down(VK_SHIFT.0))
                    && (!shortcut.platform || down(VK_LWIN.0) || down(VK_RWIN.0));
                let next = chord && (held || pressed);
                if next != held {
                    held = next;
                    let _ = tx.unbounded_send(Event::Held(held));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = UnregisterHotKey(None, ID);
            let _ = tx.unbounded_send(Event::Held(false));
        })
        .map_err(|e| e.to_string())?;
    Ok((Registration(cancel), rx))
}

fn virtual_key(key: &str) -> Option<u16> {
    Some(match key {
        "space" => VK_SPACE.0,
        "tab" => VK_TAB.0,
        "enter" => VK_RETURN.0,
        "backspace" => VK_BACK.0,
        "delete" => VK_DELETE.0,
        "insert" => VK_INSERT.0,
        "up" => VK_UP.0,
        "down" => VK_DOWN.0,
        "left" => VK_LEFT.0,
        "right" => VK_RIGHT.0,
        "home" => VK_HOME.0,
        "end" => VK_END.0,
        "pageup" => VK_PRIOR.0,
        "pagedown" => VK_NEXT.0,
        key if key.len() == 1 => u16::from(key.as_bytes()[0].to_ascii_uppercase()),
        key => VK_F1.0 + key.strip_prefix('f')?.parse::<u16>().ok()?.checked_sub(1)?,
    })
}
