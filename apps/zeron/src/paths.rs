//! Application storage paths. Provider credentials keep their own locations.

use std::ffi::OsString;
use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    resolve_data_dir(|name| std::env::var_os(name))
}

/// Integration configuration lives in the user's home, independently of data profiles.
pub fn env_file() -> Option<PathBuf> {
    resolve_env_file(|name| std::env::var_os(name), cfg!(windows))
}

fn resolve_env_file(
    mut env: impl FnMut(&str) -> Option<OsString>,
    windows: bool,
) -> Option<PathBuf> {
    // Explorer normally has USERPROFILE but no HOME. Prefer the native home on
    // Windows so shell and desktop launches use the same integration settings.
    let keys = if windows {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };
    keys.into_iter()
        .find_map(|name| env(name).filter(|value| !value.is_empty()))
        .map(|home| PathBuf::from(home).join(".zeron").join("env"))
}

fn resolve_data_dir(mut env: impl FnMut(&str) -> Option<OsString>) -> PathBuf {
    if let Some(dir) = env("ZERON_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        // Explorer does not set HOME. Do not let a shell-specific HOME select
        // a different workspace from a desktop launch, or migrate credentials
        // between Unix-style and native Windows directories implicitly.
        let local = env("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                env("USERPROFILE")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join("AppData").join("Local"))
            })
            .expect("LOCALAPPDATA and USERPROFILE not set; set ZERON_DATA_DIR");
        local.join("Zeron")
    }
    #[cfg(not(windows))]
    {
        let home = PathBuf::from(env("HOME").expect("HOME not set"));
        let dir = home.join(".zeron");
        // One-shot 0.2.0 migration: adopt the pre-rename data dir.
        if !dir.exists() {
            let old = home.join(".comet-native");
            if old.exists() && std::fs::rename(&old, &dir).is_ok() {
                eprintln!("migrated data dir {} -> {}", old.display(), dir.display());
            }
        }
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(vars: &[(&str, &str)]) -> PathBuf {
        resolve_data_dir(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.into())
        })
    }

    #[test]
    fn integration_config_uses_windows_profile_without_home() {
        let native = OsString::from("native-profile");
        let path = resolve_env_file(|name| (name == "USERPROFILE").then(|| native.clone()), true);
        assert_eq!(path, Some(PathBuf::from("native-profile/.zeron/env")));
        let path = resolve_env_file(
            |name| match name {
                "HOME" => Some("shell-home".into()),
                "USERPROFILE" => Some(native.clone()),
                _ => None,
            },
            true,
        );
        assert_eq!(path, Some(PathBuf::from("native-profile/.zeron/env")));
        assert_eq!(resolve_env_file(|_| None, true), None);
    }

    #[test]
    fn explicit_data_dir_needs_no_home() {
        assert_eq!(
            resolve(&[("ZERON_DATA_DIR", "custom data")]),
            PathBuf::from("custom data")
        );
    }

    #[cfg(windows)]
    #[test]
    fn explorer_launch_without_home_uses_local_app_data() {
        assert_eq!(
            resolve(&[("LOCALAPPDATA", r"C:\Users\Test User\AppData\Local")]),
            PathBuf::from(r"C:\Users\Test User\AppData\Local\Zeron"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_profile_fallback_handles_unicode_and_apostrophes() {
        assert_eq!(
            resolve(&[("USERPROFILE", r"C:\Users\O'Brien 日本語")]),
            PathBuf::from(r"C:\Users\O'Brien 日本語\AppData\Local\Zeron"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_default_does_not_depend_on_shell_home() {
        assert_eq!(
            resolve(&[("HOME", r"D:\msys-home"), ("LOCALAPPDATA", r"C:\Local")]),
            PathBuf::from(r"C:\Local\Zeron"),
        );
    }
}
