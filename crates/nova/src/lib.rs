//! Zeron Nova's per-install identity: every name a Nova build must not share
//! with an upstream Zeron install on the same machine. Upstream's defaults
//! point here; `dist/` files repeat the values, and `tests/identity.rs`
//! holds both to them.

/// The product name users see.
pub const APP_NAME: &str = "Zeron Nova";

/// macOS bundle identifier (`dist/macos/Info.plist`), also the launchd label
/// of `zeron daemon`.
pub const BUNDLE_ID: &str = "sh.zeron.app.nova";

/// Data directory under `$HOME` on Unix: sessions, logs, the engine lock,
/// managed adapters, worktrees and scratch chats.
pub const DATA_DIR: &str = ".zeron-nova";

/// Data directory under `%LOCALAPPDATA%` on Windows.
pub const WINDOWS_DATA_DIR: &str = "Zeron Nova";

/// The engine's loopback IPC port.
pub const IPC_PORT: u16 = 27664;

/// The literal behind [`URL_SCHEME`], for `concat!`.
#[macro_export]
macro_rules! url_scheme {
    () => {
        "zeron-nova"
    };
}

/// Conversation and notification links: `zeron-nova://…`.
pub const URL_SCHEME: &str = url_scheme!();

/// Wayland app id and X11 `WM_CLASS`; matches the installed
/// `zeron-nova.desktop`.
pub const LINUX_APP_ID: &str = "zeron-nova";

/// The systemd user unit of `zeron daemon`.
pub const SYSTEMD_UNIT: &str = "zeron-nova.service";

/// The installer's uninstall entry: `AppId` in `dist/windows/zeron.iss` plus
/// Inno Setup's `_is1` suffix.
pub const WINDOWS_UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{97089E9E-3EDD-4E6A-B5A0-2712BE6C0E9C}_is1";
