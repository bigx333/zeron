//! Every per-install identity differs from upstream Zeron's, and the files
//! that ship outside Rust carry the same values as `zeron_nova`.

use std::path::Path;

use zeron_nova::*;

fn shipped(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path))
        .unwrap_or_else(|err| panic!("{path}: {err}"))
        .replace("\r\n", "\n")
}

fn assert_has(path: &str, text: &str, needle: &str) {
    assert!(text.contains(needle), "{path} lacks {needle:?}");
}

#[test]
fn every_identity_differs_from_upstream() {
    let ipc_port = IPC_PORT.to_string();
    let pairs = [
        ("app name", "Zeron", APP_NAME),
        ("bundle id and launchd label", "sh.zeron.app", BUNDLE_ID),
        ("data dir", ".zeron", DATA_DIR),
        ("Windows data dir", "Zeron", WINDOWS_DATA_DIR),
        ("IPC port", "27654", &ipc_port),
        ("URL scheme", "zeron", URL_SCHEME),
        ("Linux app id", "zeron", LINUX_APP_ID),
        ("systemd unit", "zeron.service", SYSTEMD_UNIT),
        (
            "Windows uninstall key",
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{AD5DEC34-E254-467B-8F24-8127EBAF4DA6}_is1",
            WINDOWS_UNINSTALL_KEY,
        ),
    ];
    for (what, upstream, nova) in pairs {
        assert_ne!(upstream, nova, "{what} is upstream's");
    }
    assert!(!WINDOWS_UNINSTALL_KEY.contains("AD5DEC34"));
}

#[test]
fn macos_bundles_carry_nova_values() {
    let path = "dist/macos/Info.plist";
    let plist = shipped(path);
    assert_has(path, &plist, &format!("<string>{BUNDLE_ID}</string>"));
    assert_has(path, &plist, &format!("<string>{APP_NAME}</string>"));
    let schemes = plist
        .split("<key>CFBundleURLSchemes</key>")
        .nth(1)
        .and_then(|rest| rest.split("</array>").next())
        .unwrap_or_else(|| panic!("{path} registers no URL scheme"));
    assert_eq!(
        schemes.split_whitespace().collect::<String>(),
        format!("<array><string>{URL_SCHEME}</string>"),
        "{path} must claim only Nova's scheme"
    );
    assert_has(
        path,
        &plist,
        &format!("<string>{BUNDLE_ID}.conversation</string>"),
    );
    assert!(!plist.contains("<string>sh.zeron.app</string>"));

    let path = "dist/macos/Info-dev.plist";
    let dev = shipped(path);
    assert_has(path, &dev, &format!("<string>{BUNDLE_ID}.dev</string>"));
    let path = "scripts/run-macos-dev.sh";
    let runner = shipped(path);
    let dev_port = runner
        .split("ZERON_DEV_IPC_PORT:-")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .unwrap_or_else(|| panic!("{path} has no default dev IPC port"));
    assert_ne!(dev_port, IPC_PORT.to_string());
    assert_ne!(dev_port, "49777", "upstream's development bundle port");
}

#[test]
fn linux_entry_and_installer_carry_nova_values() {
    let path = "dist/zeron.desktop";
    let entry = shipped(path);
    assert_has(path, &entry, &format!("\nName={APP_NAME}\n"));
    assert_has(path, &entry, &format!("\nStartupWMClass={LINUX_APP_ID}\n"));
    assert_has(
        path,
        &entry,
        &format!("\nMimeType=x-scheme-handler/{URL_SCHEME};\n"),
    );

    let path = "scripts/package-linux.sh";
    let package = shipped(path);
    assert_has(
        path,
        &package,
        &format!("APP_ROOT=\"$HOME/{DATA_DIR}/app\"\n"),
    );
    assert_has(
        path,
        &package,
        &format!("\"$STAGE/{LINUX_APP_ID}.desktop\""),
    );
    assert_has(
        path,
        &package,
        &format!("\"$apps_dir/{LINUX_APP_ID}.desktop\""),
    );
    assert_has(path, &package, &format!("\"$icon_dir/{LINUX_APP_ID}.png\""));
    assert_has(
        path,
        &package,
        &format!("\"$HOME/.local/bin/{LINUX_APP_ID}\""),
    );
    assert!(
        !package.contains("/.zeron/"),
        "{path} writes upstream's ~/.zeron"
    );
}

#[test]
fn windows_installer_carries_nova_values() {
    let path = "dist/windows/zeron.iss";
    let iss = shipped(path);
    let guid = WINDOWS_UNINSTALL_KEY
        .rsplit('\\')
        .next()
        .and_then(|key| key.strip_suffix("_is1"))
        .unwrap();
    assert_has(path, &iss, &format!("\nAppId={{{guid}\n"));
    assert_has(path, &iss, &format!("#define AppName \"{APP_NAME}\"\n"));
    assert_has(path, &iss, &format!("#define UrlScheme \"{URL_SCHEME}\"\n"));
    assert_has(path, &iss, "\nDefaultDirName={autopf}\\{#AppName}\n");
    let classes: Vec<&str> = iss
        .lines()
        .filter(|line| line.contains(r"Software\Classes\"))
        .collect();
    assert_eq!(classes.len(), 4, "{path} URL class rows");
    for line in classes {
        assert!(line.contains(r#""Software\Classes\{#UrlScheme}"#), "{line}");
    }

    let path = "scripts/test-windows-installer.ps1";
    let check = shipped(path);
    assert_has(path, &check, &format!("'HKCU:\\{WINDOWS_UNINSTALL_KEY}'"));
    assert_has(
        path,
        &check,
        &format!("'HKCU:\\Software\\Classes\\{URL_SCHEME}'"),
    );
}
