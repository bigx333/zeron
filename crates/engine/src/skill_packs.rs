//! Skill packs: git repositories that carry many skills (pstack ships about
//! sixty under `plugins/pstack/skills`). A pack is cloned whole into
//! `~/.zeron/packs/<name>`, so skills that reference each other by relative
//! path and ship scripts keep working, and each of its skills is linked into
//! the shared library at `~/.zeron/skills/<skill>`, where every harness
//! already loads it.
//!
//! Packs follow releases, not a branch: a repository with version tags moves
//! tag to tag; one without moves only when its `VERSION` (or plugin manifest
//! version) changes. Each engine keeps its own copy current on its own
//! device, in the background, for the packs that opt in.
//!
//! A pack is third-party code agents will run. Zeron never runs any of it:
//! git runs with hooks disabled, no submodules, no prompts and no local or
//! `ext::` transports, and installing or updating only clones, checks out and
//! links. Skills whose names are already taken are skipped and reported, not
//! shadowed.
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "packs.json";
const INSPECTION_FILE: &str = "inspection.json";
const SKILL_FILE: &str = "SKILL.md";
const GIT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// An inspected but uninstalled pack is dropped after this long.
const STAGING_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_CHANGES_CHARS: usize = 12_000;
const MAX_PACK_SKILLS: usize = 500;

const FIRST_CHECK: Duration = Duration::from_secs(2 * 60);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_JITTER_SECS: u64 = 30 * 60;
const FIRST_RETRY: Duration = Duration::from_secs(15 * 60);

/// Where packs live relative to the shared skill library.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `~/.zeron/skills`, where pack skills are linked.
    pub skills: PathBuf,
    /// `~/.zeron/packs`: one checkout per pack, and their records.
    pub packs: PathBuf,
    /// `~/.zeron/.packs-staging`: inspected clones awaiting install.
    pub staging: PathBuf,
}

impl Paths {
    pub fn for_library(skills: &Path) -> Self {
        let zeron = skills.parent().unwrap_or(skills);
        Self {
            skills: skills.to_owned(),
            packs: zeron.join("packs"),
            staging: zeron.join(".packs-staging"),
        }
    }

    pub fn current() -> Self {
        Self::for_library(&zeron_harness::shared_skills_root())
    }

    fn checkout(&self, name: &str) -> PathBuf {
        self.packs.join(name)
    }
}

/// How a pack finds its next release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    /// The newest version tag (`v1.2.3`, `1.2`).
    Tags,
    /// The default branch, moving only when its declared version changes.
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackUpdate {
    /// The tag, or the branch for [`Channel::Version`].
    pub reference: String,
    pub commit: String,
    pub version: Option<String>,
    /// The changelog's entry for this version, as Markdown.
    pub changes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPack {
    pub name: String,
    /// As entered: `owner/repo` or a git URL.
    pub source: String,
    pub url: String,
    pub subpath: Option<String>,
    pub channel: Channel,
    /// The checked-out tag, or the followed branch.
    pub reference: String,
    pub commit: String,
    pub version: Option<String>,
    /// Folders holding the pack's skills, relative to the checkout.
    pub skills_roots: Vec<String>,
    /// Plugin folders (with `.claude-plugin/plugin.json`), relative to the
    /// checkout. Recorded for handing agents and hooks to Claude later.
    pub plugin_dirs: Vec<String>,
    /// Skills linked into the library.
    pub skills: Vec<String>,
    /// Skills not linked because the name was already taken.
    pub skipped: Vec<String>,
    pub auto_update: bool,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub checked_at: Option<DateTime<Utc>>,
    pub update: Option<PackUpdate>,
    /// The last background check or update failure, until one succeeds.
    pub last_error: Option<String>,
    /// The checkout; filled when read, not stored.
    #[serde(default, skip_deserializing)]
    pub directory: String,
}

/// A cloned pack awaiting confirmation: what installing it would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInspection {
    pub token: String,
    pub name: String,
    pub source: String,
    pub url: String,
    pub subpath: Option<String>,
    pub channel: Channel,
    pub reference: String,
    pub commit: String,
    pub version: Option<String>,
    pub skills_roots: Vec<String>,
    pub plugin_dirs: Vec<String>,
    /// Every skill the pack carries.
    pub skills: Vec<String>,
    /// Those whose names are already taken in the library; installing skips them.
    pub clashes: Vec<String>,
}

/// Serializes pack mutations: installs, updates and removals, whether they
/// come from the page or the background updater.
static OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- records ----

pub fn load(paths: &Paths) -> Vec<SkillPack> {
    let Ok(text) = std::fs::read_to_string(paths.packs.join(STATE_FILE)) else {
        return Vec::new();
    };
    let mut packs: Vec<SkillPack> = serde_json::from_str(&text).unwrap_or_default();
    for pack in &mut packs {
        pack.directory = paths.checkout(&pack.name).to_string_lossy().into_owned();
    }
    packs.sort_by_key(|pack| pack.name.to_lowercase());
    packs
}

fn save(paths: &Paths, packs: &[SkillPack]) -> Result<(), String> {
    std::fs::create_dir_all(&paths.packs).map_err(|error| error.to_string())?;
    let temp = paths
        .packs
        .join(format!(".{STATE_FILE}.{}", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(packs).map_err(|error| error.to_string())?;
    std::fs::write(&temp, bytes)
        .and_then(|()| std::fs::rename(&temp, paths.packs.join(STATE_FILE)))
        .map_err(|error| {
            let _ = std::fs::remove_file(&temp);
            format!("Couldn't save the pack list: {error}")
        })
}

/// The pack whose checkout a library link points into, if any.
pub fn owner_of_link(paths: &Paths, link: &Path) -> Option<String> {
    let target = std::fs::canonicalize(link).ok()?;
    let packs = std::fs::canonicalize(&paths.packs).ok()?;
    let relative = target.strip_prefix(&packs).ok()?;
    let name = relative.components().next()?.as_os_str().to_str()?;
    (!name.starts_with('.')).then(|| name.to_owned())
}

// ---- sources ----

/// The clone URL and pack name for `source`: `owner/repo`, a GitHub address,
/// an `https://` URL or an `git@host:owner/repo` address. Local paths and
/// other transports are refused.
fn parse_source(source: &str) -> Result<(String, String), String> {
    let source = source.trim().trim_end_matches('/');
    let invalid = || {
        format!(
            "“{source}” isn't a git repository Zeron can install. Use owner/repo or an https:// URL."
        )
    };
    if source.is_empty() || source.starts_with('-') || source.chars().any(char::is_whitespace) {
        return Err(invalid());
    }
    let safe = |segment: &str| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    let url = if let Some(rest) = source
        .strip_prefix("https://")
        .or_else(|| source.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/').ok_or_else(invalid)?;
        if host.is_empty() || !path.split('/').all(safe) {
            return Err(invalid());
        }
        format!("https://{host}/{path}")
    } else if let Some(rest) = source.strip_prefix("git@") {
        let (host, path) = rest.split_once(':').ok_or_else(invalid)?;
        if host.is_empty() || !path.split('/').all(safe) {
            return Err(invalid());
        }
        format!("git@{host}:{path}")
    } else {
        let path = source.strip_prefix("github.com/").unwrap_or(source);
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 2 || !parts.iter().all(|part| safe(part)) || parts[0].contains('.') {
            return Err(invalid());
        }
        format!("https://github.com/{}/{}", parts[0], parts[1])
    };
    let name = url
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git")
        .to_lowercase();
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-').to_owned();
    if name.is_empty() {
        return Err(invalid());
    }
    Ok((url, name))
}

fn parse_subpath(subpath: Option<&str>) -> Result<Option<String>, String> {
    let Some(subpath) = subpath
        .map(|value| value.trim().trim_matches('/'))
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let safe = subpath
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
        && !subpath.contains(['\\', ':', '\0']);
    if !safe {
        return Err(format!("“{subpath}” isn't a folder inside the repository."));
    }
    Ok(Some(subpath.to_owned()))
}

// ---- git ----

const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// The user's `credential.*` config, in order and with its host scopes and
/// empty resets, read once outside any repository. Network commands get it
/// so private packs still authenticate; nothing else from the user's config
/// (filters, fsmonitor, ssh wrappers) reaches git.
async fn credential_config() -> &'static [(String, Option<String>)] {
    static CONFIG: tokio::sync::OnceCell<Vec<(String, Option<String>)>> =
        tokio::sync::OnceCell::const_new();
    CONFIG
        .get_or_init(|| async {
            let mut cmd = tokio::process::Command::new("git");
            zeron_harness::compose_login_shell_path(&mut cmd);
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("GIT_") {
                    cmd.env_remove(key);
                }
            }
            cmd.args(["config", "--null", "--get-regexp", r"^credential\."])
                .current_dir(std::env::temp_dir())
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            let output = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await;
            match output {
                Ok(Ok(output)) if output.status.success() => parse_config_list(&output.stdout),
                _ => Vec::new(),
            }
        })
        .await
}

/// `git config --null` output: `key\nvalue\0` per entry, or `key\0` for a
/// valueless (boolean) key.
fn parse_config_list(bytes: &[u8]) -> Vec<(String, Option<String>)> {
    String::from_utf8_lossy(bytes)
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.split_once('\n') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (entry.to_owned(), None),
        })
        .collect()
}

/// Run git on `repo` (or none, for a clone). Nothing from the repository
/// runs: inherited git environment and the system and global config are
/// dropped, so no configured filter, hook or helper can be switched on by the
/// pack's `.gitattributes`, and symlinks are checked out as plain files.
///
/// Only `network` commands (clone, fetch) get the user's credential config,
/// and they run from an empty folder with the repository given as
/// `GIT_DIR`, so a helper written as a relative command can never resolve to
/// a script inside the pack. Clones are full, so local commands never fetch.
async fn git_in(repo: Option<&Path>, network: bool, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("git");
    zeron_harness::compose_login_shell_path(&mut cmd);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE);
    cmd.args([
        "-c",
        &format!("core.hooksPath={NULL_DEVICE}"),
        "-c",
        "core.symlinks=false",
        "-c",
        "submodule.recurse=false",
        "-c",
        "protocol.file.allow=never",
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "core.fsmonitor=false",
    ]);
    let empty = if network {
        for (key, value) in credential_config().await {
            cmd.arg("-c").arg(match value {
                Some(value) => format!("{key}={value}"),
                None => key.clone(),
            });
        }
        let empty = tempfile::tempdir().map_err(|error| format!("git couldn't start: {error}"))?;
        cmd.current_dir(empty.path());
        if let Some(repo) = repo {
            cmd.env("GIT_DIR", repo.join(".git"));
        }
        Some(empty)
    } else {
        if let Some(repo) = repo {
            cmd.current_dir(repo);
        }
        None
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.as_std_mut().creation_flags(0x08000000);
        cmd.args(["-c", "core.longpaths=true"]);
    }
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GCM_INTERACTIVE", "never")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(GIT_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "git took too long and was stopped.".to_string())?
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                "Installing packs needs git. Install it and try again.".to_string()
            }
            _ => format!("git couldn't start: {error}"),
        })?;
    drop(empty);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("git failed")
            .trim_start_matches("fatal: ")
            .to_owned();
        return Err(message);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// A local git command on `repo`; it never reaches the network.
async fn git(repo: Option<&Path>, args: &[&str]) -> Result<String, String> {
    git_in(repo, false, args).await
}

async fn clone(url: &str, dest: &Path) -> Result<(), String> {
    let dest = dest.to_string_lossy();
    git_in(
        None,
        true,
        &["clone", "--quiet", "--no-checkout", "--", url, &dest],
    )
    .await
    .map(drop)
    .map_err(|error| format!("Couldn't clone {url}: {error}"))
}

/// Release tags, newest first: `v1.2.3`, `1.2`, never pre-releases. Ordered
/// by their numbers, so `2.0.0` outranks `v1.9.0` whatever the prefix.
async fn release_tags(checkout: &Path) -> Result<Vec<String>, String> {
    let tags = git(Some(checkout), &["tag", "--list"]).await?;
    let mut releases: Vec<(Vec<u64>, String)> = tags
        .lines()
        .filter_map(|tag| release_key(tag).map(|key| (key, tag.to_owned())))
        .collect();
    releases.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    Ok(releases.into_iter().map(|(_, tag)| tag).collect())
}

/// A release tag's numbers, zero-padded for comparison: `v1.2` → `[1, 2, 0, 0]`.
fn release_key(tag: &str) -> Option<Vec<u64>> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = version.split('.').collect();
    if version.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut key = parts
        .into_iter()
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u64>().ok())
                .flatten()
        })
        .collect::<Option<Vec<u64>>>()?;
    key.resize(4, 0);
    Some(key)
}

#[cfg(test)]
fn is_release_tag(tag: &str) -> bool {
    release_key(tag).is_some()
}

async fn default_branch(checkout: &Path) -> Result<String, String> {
    let head = git(
        Some(checkout),
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await?;
    Ok(head.trim_start_matches("origin/").to_owned())
}

async fn commit_of(checkout: &Path, reference: &str) -> Result<String, String> {
    git(
        Some(checkout),
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )
    .await
}

async fn file_at(checkout: &Path, reference: &str, path: &str) -> Option<String> {
    git(Some(checkout), &["show", &format!("{reference}:{path}")])
        .await
        .ok()
}

async fn checkout_detached(checkout: &Path, commit: &str) -> Result<(), String> {
    git(Some(checkout), &["checkout", "--quiet", "--detach", commit])
        .await
        .map(drop)
        .map_err(|error| {
            if error.contains("overwritten") || error.contains("local changes") {
                "Files in the pack were edited, so it can't update without losing them. Remove the pack and install it again, or undo the edits.".into()
            } else {
                format!("Couldn't check out {commit}: {error}")
            }
        })
}

// ---- layout ----

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Layout {
    skills_roots: Vec<String>,
    plugin_dirs: Vec<String>,
}

fn holds_skills(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|entry| entry.path().join(SKILL_FILE).is_file())
    })
}

/// `path` resolves inside `checkout` (both canonicalized; `/tmp` and
/// `/private/tmp` are one place on macOS).
fn within(checkout: &Path, path: &Path) -> bool {
    std::fs::canonicalize(path)
        .ok()
        .zip(std::fs::canonicalize(checkout).ok())
        .is_some_and(|(path, checkout)| path.starts_with(checkout))
}

fn relative(checkout: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(checkout).unwrap_or(path);
    let text = relative.to_string_lossy().replace('\\', "/");
    if text.is_empty() { ".".into() } else { text }
}

/// Where a pack keeps its skills, under `base` (the checkout or its subpath):
/// a marketplace's plugins, then `plugins/*/skills`, then `skills`, then
/// skill folders directly in `base`.
fn detect(checkout: &Path, base: &Path) -> Layout {
    let mut layout = Layout::default();
    let add_plugin = |layout: &mut Layout, plugin: &Path| {
        let skills = plugin.join("skills");
        if within(checkout, &skills) && holds_skills(&skills) {
            layout.skills_roots.push(relative(checkout, &skills));
            if plugin.join(".claude-plugin/plugin.json").is_file() {
                layout.plugin_dirs.push(relative(checkout, plugin));
            }
        }
    };
    if let Ok(text) = std::fs::read_to_string(base.join(".claude-plugin/marketplace.json"))
        && let Ok(marketplace) = serde_json::from_str::<serde_json::Value>(&text)
    {
        for plugin in marketplace
            .get("plugins")
            .and_then(|plugins| plugins.as_array())
            .into_iter()
            .flatten()
        {
            let Some(source) = plugin.get("source").and_then(|source| source.as_str()) else {
                continue;
            };
            // Join the validated path, never the raw one: `/elsewhere`
            // validates as `elsewhere` but would join as absolute.
            if let Ok(Some(source)) = parse_subpath(Some(source.trim_start_matches("./"))) {
                add_plugin(&mut layout, &base.join(source));
            }
        }
    }
    if layout.skills_roots.is_empty()
        && let Ok(entries) = std::fs::read_dir(base.join("plugins"))
    {
        let mut plugins: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
        plugins.sort();
        for plugin in plugins {
            add_plugin(&mut layout, &plugin);
        }
    }
    if layout.skills_roots.is_empty()
        && within(checkout, &base.join("skills"))
        && holds_skills(&base.join("skills"))
    {
        add_plugin(&mut layout, base);
        if layout.skills_roots.is_empty() {
            layout
                .skills_roots
                .push(relative(checkout, &base.join("skills")));
        }
    }
    if layout.skills_roots.is_empty() && within(checkout, base) && holds_skills(base) {
        layout.skills_roots.push(relative(checkout, base));
    }
    layout
}

/// Every skill under the layout's roots as `(link name, folder)`; the first
/// root to name a skill wins.
fn pack_skills(checkout: &Path, layout: &Layout) -> Vec<(String, PathBuf)> {
    let mut skills = BTreeMap::new();
    for root in &layout.skills_roots {
        let root = checkout.join(root);
        if !within(checkout, &root) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut folders: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        folders.sort();
        for folder in folders {
            let Some(name) = folder.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let linkable = !name.starts_with('.')
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            // A real file inside the checkout: a manifest that is a link
            // could show text from anywhere as the skill's instructions.
            let manifest = folder.join(SKILL_FILE);
            let contained = std::fs::symlink_metadata(&manifest)
                .is_ok_and(|meta| meta.file_type().is_file())
                && within(checkout, &folder);
            if linkable
                && contained
                && zeron_harness::read_skill_manifest(&manifest).is_some()
                && skills.len() < MAX_PACK_SKILLS
            {
                skills.entry(name.to_owned()).or_insert(folder);
            }
        }
    }
    skills.into_iter().collect()
}

/// The version the pack declares: a `VERSION` file at its base, else a
/// plugin manifest's `version`.
fn declared_version(checkout: &Path, base: &Path, layout: &Layout) -> Option<String> {
    let version_file = std::fs::read_to_string(base.join("VERSION"))
        .ok()
        .and_then(|text| text.lines().next().map(|line| line.trim().to_owned()))
        .filter(|line| !line.is_empty() && line.len() <= 64);
    version_file.or_else(|| {
        layout.plugin_dirs.iter().find_map(|plugin| {
            let text =
                std::fs::read_to_string(checkout.join(plugin).join(".claude-plugin/plugin.json"))
                    .ok()?;
            let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
            manifest
                .get("version")?
                .as_str()
                .map(str::to_owned)
                .filter(|version| !version.is_empty() && version.len() <= 64)
        })
    })
}

/// The version declared at `reference` without checking it out.
async fn version_at(checkout: &Path, reference: &str, pack: &SkillPack) -> Option<String> {
    let base = pack
        .subpath
        .as_deref()
        .map(|subpath| format!("{subpath}/"))
        .unwrap_or_default();
    if let Some(text) = file_at(checkout, reference, &format!("{base}VERSION")).await
        && let Some(line) = text
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    {
        return Some(line.to_owned());
    }
    for plugin in &pack.plugin_dirs {
        if let Some(text) = file_at(
            checkout,
            reference,
            &format!("{plugin}/.claude-plugin/plugin.json"),
        )
        .await
            && let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(version) = manifest.get("version").and_then(|v| v.as_str())
        {
            return Some(version.to_owned());
        }
    }
    None
}

/// The changelog section for `version`: from its heading to the next heading
/// of the same or a higher level.
fn changes_entry(text: &str, version: &str) -> Option<String> {
    let version = version.trim_start_matches('v');
    let heading = |line: &str| {
        let level = line.bytes().take_while(|b| *b == b'#').count();
        (level > 0 && line.as_bytes().get(level) == Some(&b' ')).then_some(level)
    };
    let mentions = |line: &str| {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
            .any(|word| word.trim_start_matches('v') == version)
    };
    let mut lines = text.lines();
    let level = loop {
        let line = lines.next()?;
        if let Some(level) = heading(line)
            && level > 1
            && mentions(line)
        {
            break level;
        }
    };
    let mut entry = String::new();
    for line in lines {
        if heading(line).is_some_and(|other| other <= level) {
            break;
        }
        entry.push_str(line);
        entry.push('\n');
    }
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let mut entry = entry.to_owned();
    if entry.len() > MAX_CHANGES_CHARS {
        let cut = (0..=MAX_CHANGES_CHARS)
            .rev()
            .find(|ix| entry.is_char_boundary(*ix))
            .unwrap_or(0);
        entry.truncate(cut);
        entry.push('…');
    }
    Some(entry)
}

async fn changes_at(
    checkout: &Path,
    reference: &str,
    pack: &SkillPack,
    version: &str,
) -> Option<String> {
    let base = pack
        .subpath
        .as_deref()
        .map(|subpath| format!("{subpath}/"))
        .unwrap_or_default();
    for name in ["CHANGES.md", "CHANGELOG.md"] {
        if let Some(text) = file_at(checkout, reference, &format!("{base}{name}")).await {
            return changes_entry(&text, version);
        }
    }
    None
}

// ---- links ----

/// Is `link` a link into `checkout`?
fn links_into(link: &Path, checkout: &Path) -> bool {
    if !std::fs::symlink_metadata(link).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return false;
    }
    let Ok(target) = std::fs::read_link(link) else {
        return false;
    };
    let target = if target.is_absolute() {
        target
    } else {
        link.parent().unwrap_or(Path::new("")).join(target)
    };
    target.starts_with(checkout)
        || std::fs::canonicalize(&target)
            .ok()
            .zip(std::fs::canonicalize(checkout).ok())
            .is_some_and(|(target, checkout)| target.starts_with(checkout))
}

fn make_link(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    return std::os::windows::fs::symlink_dir(target, link);
}

fn unlink(link: &Path) {
    if std::fs::remove_file(link).is_err() {
        let _ = std::fs::remove_dir(link);
    }
}

/// A link change `relink` made, kept so a failure can undo it.
enum LinkChange {
    Created(PathBuf),
    Removed { link: PathBuf, target: PathBuf },
}

/// Link `skills` into the library, replacing only this pack's own links.
/// Returns what was linked and what was skipped for a taken name; links this
/// pack made for skills no longer listed are removed. All or nothing: on an
/// error every change made so far is undone.
fn relink(
    paths: &Paths,
    checkout: &Path,
    previous: &[String],
    skills: &[(String, PathBuf)],
) -> Result<(Vec<String>, Vec<String>), String> {
    std::fs::create_dir_all(&paths.skills)
        .map_err(|error| format!("Couldn't create {}: {error}", paths.skills.display()))?;
    let mut changes = Vec::new();
    let result = relink_with(paths, checkout, previous, skills, &mut changes);
    if result.is_err() {
        for change in changes.into_iter().rev() {
            match change {
                LinkChange::Created(link) => unlink(&link),
                LinkChange::Removed { link, target } => {
                    let _ = make_link(&target, &link);
                }
            }
        }
    }
    result
}

fn relink_with(
    paths: &Paths,
    checkout: &Path,
    previous: &[String],
    skills: &[(String, PathBuf)],
    changes: &mut Vec<LinkChange>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let remove = |link: PathBuf, changes: &mut Vec<LinkChange>| -> Result<(), String> {
        let target = std::fs::read_link(&link).map_err(|error| error.to_string())?;
        std::fs::remove_file(&link)
            .or_else(|_| std::fs::remove_dir(&link))
            .map_err(|error| format!("Couldn't unlink {}: {error}", link.display()))?;
        changes.push(LinkChange::Removed { link, target });
        Ok(())
    };
    let current: HashSet<&str> = skills.iter().map(|(name, _)| name.as_str()).collect();
    for name in previous {
        let link = paths.skills.join(name);
        if !current.contains(name.as_str()) && links_into(&link, checkout) {
            remove(link, changes)?;
        }
    }
    let mut linked = Vec::new();
    let mut skipped = Vec::new();
    for (name, folder) in skills {
        let link = paths.skills.join(name);
        if std::fs::symlink_metadata(&link).is_ok() {
            if !links_into(&link, checkout) {
                skipped.push(name.clone());
                continue;
            }
            if std::fs::read_link(&link).is_ok_and(|target| &target == folder) {
                linked.push(name.clone());
                continue;
            }
            remove(link.clone(), changes)?;
        }
        make_link(folder, &link).map_err(|error| format!("Couldn't link {name}: {error}"))?;
        changes.push(LinkChange::Created(link));
        linked.push(name.clone());
    }
    Ok((linked, skipped))
}

// ---- inspect / install ----

fn sweep_staging(paths: &Paths) {
    let Ok(entries) = std::fs::read_dir(&paths.staging) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_none_or(|age| age > STAGING_TTL);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn clashes(paths: &Paths, skills: &[(String, PathBuf)]) -> Vec<String> {
    skills
        .iter()
        .filter(|(name, _)| std::fs::symlink_metadata(paths.skills.join(name)).is_ok())
        .map(|(name, _)| name.clone())
        .collect()
}

pub async fn inspect(source: String, subpath: Option<String>) -> Result<PackInspection, String> {
    inspect_at(&Paths::current(), &source, subpath.as_deref()).await
}

async fn inspect_at(
    paths: &Paths,
    source: &str,
    subpath: Option<&str>,
) -> Result<PackInspection, String> {
    let (url, name) = parse_source(source)?;
    let subpath = parse_subpath(subpath)?;
    if load(paths).iter().any(|pack| pack.name == name) {
        return Err(format!("{name} is already installed."));
    }
    sweep_staging(paths);
    let token = uuid::Uuid::new_v4().to_string();
    let stage = paths.staging.join(&token);
    let checkout = stage.join("checkout");
    tokio::fs::create_dir_all(&stage)
        .await
        .map_err(|error| format!("Couldn't prepare the install: {error}"))?;
    let result = async {
        clone(&url, &checkout).await?;
        let tags = release_tags(&checkout).await?;
        let (channel, reference) = match tags.first() {
            Some(tag) => (Channel::Tags, tag.clone()),
            None => (Channel::Version, default_branch(&checkout).await?),
        };
        let commit = match channel {
            Channel::Tags => commit_of(&checkout, &reference).await?,
            Channel::Version => commit_of(&checkout, &format!("origin/{reference}")).await?,
        };
        checkout_detached(&checkout, &commit).await?;
        let base = subpath
            .as_deref()
            .map_or(checkout.clone(), |sub| checkout.join(sub));
        if !base.is_dir() {
            return Err(format!(
                "{} has no folder named {}.",
                url,
                subpath.as_deref().unwrap_or_default()
            ));
        }
        let layout = detect(&checkout, &base);
        let skills = pack_skills(&checkout, &layout);
        if skills.is_empty() {
            return Err(format!(
                "No skills were found in {url}. A pack keeps them as skills/<name>/SKILL.md."
            ));
        }
        let version = declared_version(&checkout, &base, &layout).or_else(|| {
            (channel == Channel::Tags).then(|| reference.trim_start_matches('v').to_owned())
        });
        let inspection = PackInspection {
            token: token.clone(),
            name: name.clone(),
            source: source.trim().to_owned(),
            url: url.clone(),
            subpath: subpath.clone(),
            channel,
            reference,
            commit,
            version,
            skills_roots: layout.skills_roots,
            plugin_dirs: layout.plugin_dirs,
            clashes: clashes(paths, &skills),
            skills: skills.into_iter().map(|(name, _)| name).collect(),
        };
        tokio::fs::write(
            stage.join(INSPECTION_FILE),
            serde_json::to_vec(&inspection).map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(inspection)
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&stage).await;
    }
    result
}

pub async fn install(token: String, auto_update: bool) -> Result<SkillPack, String> {
    install_at(&Paths::current(), &token, auto_update).await
}

async fn install_at(paths: &Paths, token: &str, auto_update: bool) -> Result<SkillPack, String> {
    let _guard = OPERATIONS.lock().await;
    if uuid::Uuid::parse_str(token).is_err() {
        return Err("That install has expired. Inspect the pack again.".into());
    }
    let stage = paths.staging.join(token);
    let inspection: PackInspection = std::fs::read(stage.join(INSPECTION_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| "That install has expired. Inspect the pack again.".to_string())?;
    let mut packs = load(paths);
    if packs.iter().any(|pack| pack.name == inspection.name) {
        return Err(format!("{} is already installed.", inspection.name));
    }
    let checkout = paths.checkout(&inspection.name);
    if std::fs::symlink_metadata(&checkout).is_ok() {
        return Err(format!(
            "{} already exists. Move it aside to install this pack.",
            checkout.display()
        ));
    }
    std::fs::create_dir_all(&paths.packs).map_err(|error| error.to_string())?;
    std::fs::rename(stage.join("checkout"), &checkout)
        .map_err(|error| format!("Couldn't install {}: {error}", inspection.name))?;
    let _ = std::fs::remove_dir_all(&stage);
    let layout = Layout {
        skills_roots: inspection.skills_roots.clone(),
        plugin_dirs: inspection.plugin_dirs.clone(),
    };
    let skills = pack_skills(&checkout, &layout);
    let (linked, skipped) = match relink(paths, &checkout, &[], &skills) {
        Ok(result) => result,
        Err(error) => {
            // `relink` undid its links; drop the checkout too.
            let _ = std::fs::remove_dir_all(&checkout);
            return Err(error);
        }
    };
    let now = Utc::now();
    let pack = SkillPack {
        name: inspection.name,
        source: inspection.source,
        url: inspection.url,
        subpath: inspection.subpath,
        channel: inspection.channel,
        reference: inspection.reference,
        commit: inspection.commit,
        version: inspection.version,
        skills_roots: layout.skills_roots,
        plugin_dirs: layout.plugin_dirs,
        skills: linked,
        skipped,
        auto_update,
        installed_at: now,
        updated_at: now,
        checked_at: Some(now),
        update: None,
        last_error: None,
        directory: checkout.to_string_lossy().into_owned(),
    };
    packs.push(pack.clone());
    if let Err(error) = save(paths, &packs) {
        // Unrecorded, the pack could be neither managed nor reinstalled.
        let _ = relink(paths, &checkout, &pack.skills, &[]);
        let _ = std::fs::remove_dir_all(&checkout);
        return Err(error);
    }
    Ok(pack)
}

// ---- update ----

/// Turn a pack cloned partially (by an earlier Zeron) into a full clone,
/// through the authenticated network path, so local commands never need to
/// fetch a missing blob without credentials.
async fn ensure_full_clone(checkout: &Path) -> Result<(), String> {
    // Any marker counts: the promisor ones are cleared only after the full
    // fetch succeeds, so an interrupted migration is retried next time.
    let filtered = git(
        Some(checkout),
        &["config", "--get", "remote.origin.partialclonefilter"],
    )
    .await
    .is_ok();
    let mut partial = filtered;
    for key in ["remote.origin.promisor", "extensions.partialclone"] {
        partial |= git(Some(checkout), &["config", "--get", key]).await.is_ok();
    }
    if !partial {
        return Ok(());
    }
    // Without the filter the refetch downloads everything a full clone has;
    // a filter that can't be removed (a locked config) must stop here, or
    // the refetch would quietly stay partial.
    if filtered {
        git(
            Some(checkout),
            &["config", "--unset-all", "remote.origin.partialclonefilter"],
        )
        .await
        .map_err(|error| format!("Couldn't finish downloading the pack: {error}"))?;
    }
    git_in(
        Some(checkout),
        true,
        &["fetch", "--quiet", "--refetch", "--tags", "--force", "origin"],
    )
    .await
    .map_err(|error| format!("Couldn't finish downloading the pack: {error}"))?;
    for key in ["remote.origin.promisor", "extensions.partialclone"] {
        let _ = git(Some(checkout), &["config", "--unset-all", key]).await;
    }
    Ok(())
}

/// Fetch and record whether a newer release exists; never moves the checkout.
async fn check(paths: &Paths, pack: &mut SkillPack) -> Result<(), String> {
    let checkout = paths.checkout(&pack.name);
    ensure_full_clone(&checkout).await?;
    git_in(
        Some(&checkout),
        true,
        &["fetch", "--quiet", "--tags", "--force", "--prune", "origin"],
    )
    .await
    .map_err(|error| format!("Couldn't reach {}: {error}", pack.url))?;
    pack.checked_at = Some(Utc::now());
    pack.update = match pack.channel {
        Channel::Tags => match release_tags(&checkout).await?.into_iter().next() {
            Some(tag)
                if tag != pack.reference
                    && release_key(&tag) > release_key(&pack.reference) =>
            {
                let commit = commit_of(&checkout, &tag).await?;
                let version = version_at(&checkout, &tag, pack)
                    .await
                    .or_else(|| Some(tag.trim_start_matches('v').to_owned()));
                Some(PackUpdate {
                    changes: changes_at(&checkout, &tag, pack, version.as_deref().unwrap_or(&tag))
                        .await,
                    reference: tag,
                    commit,
                    version,
                })
            }
            _ => None,
        },
        Channel::Version => {
            let remote = format!("origin/{}", pack.reference);
            let commit = commit_of(&checkout, &remote).await?;
            let version = version_at(&checkout, &remote, pack).await;
            match version {
                Some(version)
                    if Some(&version) != pack.version.as_ref() && commit != pack.commit =>
                {
                    Some(PackUpdate {
                        changes: changes_at(&checkout, &remote, pack, &version).await,
                        reference: pack.reference.clone(),
                        commit,
                        version: Some(version),
                    })
                }
                // A pack that declares no version at all has only commits to
                // go by; they are offered, and auto-update stays off for it
                // unless chosen (see `unversioned`).
                None if pack.version.is_none() && commit != pack.commit => Some(PackUpdate {
                    changes: None,
                    reference: pack.reference.clone(),
                    commit,
                    version: None,
                }),
                _ => None,
            }
        }
    };
    pack.last_error = None;
    Ok(())
}

/// Check out the recorded update and relink: new skills appear, removed
/// ones go, the rest stay.
async fn apply(paths: &Paths, pack: &mut SkillPack) -> Result<(), String> {
    let Some(update) = pack.update.clone() else {
        return Ok(());
    };
    let checkout = paths.checkout(&pack.name);
    ensure_full_clone(&checkout).await?;
    checkout_detached(&checkout, &update.commit).await?;
    let base = pack
        .subpath
        .as_deref()
        .map_or(checkout.clone(), |sub| checkout.join(sub));
    let layout = detect(&checkout, &base);
    let skills = pack_skills(&checkout, &layout);
    // `relink` is all or nothing; put the old release back with it, so the
    // record, the checkout and the links keep agreeing.
    let (linked, skipped) = match relink(paths, &checkout, &pack.skills, &skills) {
        Ok(result) => result,
        Err(error) => {
            return Err(match checkout_detached(&checkout, &pack.commit).await {
                Ok(()) => error,
                Err(restore) => format!("{error} Restoring the previous release failed too: {restore}"),
            });
        }
    };
    pack.version = declared_version(&checkout, &base, &layout).or(update.version);
    pack.reference = update.reference;
    pack.commit = update.commit;
    pack.skills_roots = layout.skills_roots;
    pack.plugin_dirs = layout.plugin_dirs;
    pack.skills = linked;
    pack.skipped = skipped;
    pack.update = None;
    pack.updated_at = Utc::now();
    pack.last_error = None;
    Ok(())
}

/// Run `operation` on the pack named `name` and save it, failures included.
async fn with_pack<F>(paths: &Paths, name: &str, operation: F) -> Result<SkillPack, String>
where
    F: AsyncFnOnce(&Paths, &mut SkillPack) -> Result<(), String>,
{
    let _guard = OPERATIONS.lock().await;
    let mut packs = load(paths);
    let pack = packs
        .iter_mut()
        .find(|pack| pack.name == name)
        .ok_or_else(|| format!("{name} isn't installed."))?;
    let before = pack.clone();
    let result = operation(paths, pack).await;
    if let Err(error) = &result {
        pack.last_error = Some(error.clone());
    }
    let after = pack.clone();
    if let Err(error) = save(paths, &packs) {
        // The record must keep describing the checkout and its links: an
        // update that can't be recorded is undone.
        if before.commit != after.commit || before.skills != after.skills {
            if let Err(restore) = restore_release(paths, &before, &after).await {
                return Err(format!("{error} Undoing the update failed too: {restore}"));
            }
        }
        return Err(error);
    }
    result.map(|()| after)
}

/// Put `before`'s release back after `after` was applied but not recorded.
async fn restore_release(paths: &Paths, before: &SkillPack, after: &SkillPack) -> Result<(), String> {
    let checkout = paths.checkout(&before.name);
    checkout_detached(&checkout, &before.commit).await?;
    let base = before
        .subpath
        .as_deref()
        .map_or(checkout.clone(), |sub| checkout.join(sub));
    // Only what was linked before: a skill skipped then for a taken name
    // stays unlinked, as the restored record says.
    let skills: Vec<_> = pack_skills(&checkout, &detect(&checkout, &base))
        .into_iter()
        .filter(|(name, _)| before.skills.contains(name))
        .collect();
    relink(paths, &checkout, &after.skills, &skills).map(drop)
}

pub async fn check_updates(name: Option<String>) -> Result<Vec<SkillPack>, String> {
    let paths = Paths::current();
    let names: Vec<String> = match name {
        Some(name) => vec![name],
        None => load(&paths).into_iter().map(|pack| pack.name).collect(),
    };
    let mut errors = Vec::new();
    for name in names {
        if let Err(error) =
            with_pack(&paths, &name, async |paths, pack| check(paths, pack).await).await
        {
            errors.push(format!("{name}: {error}"));
        }
    }
    match errors.len() {
        0 => Ok(load(&paths)),
        _ => Err(errors.join(" ")),
    }
}

pub async fn update(name: String) -> Result<SkillPack, String> {
    update_at(&Paths::current(), &name).await
}

async fn update_at(paths: &Paths, name: &str) -> Result<SkillPack, String> {
    with_pack(paths, name, async |paths, pack| {
        if pack.update.is_none() {
            check(paths, pack).await?;
        }
        apply(paths, pack).await
    })
    .await
}

pub async fn set_auto_update(name: String, enabled: bool) -> Result<SkillPack, String> {
    with_pack(&Paths::current(), &name, async |_, pack| {
        pack.auto_update = enabled;
        Ok(())
    })
    .await
}

pub async fn remove(name: String) -> Result<(), String> {
    remove_at(&Paths::current(), &name).await
}

async fn remove_at(paths: &Paths, name: &str) -> Result<(), String> {
    let _guard = OPERATIONS.lock().await;
    let mut packs = load(paths);
    let index = packs
        .iter()
        .position(|pack| pack.name == name)
        .ok_or_else(|| format!("{name} isn't installed."))?;
    let pack = packs.remove(index);
    let checkout = paths.checkout(&pack.name);
    relink(paths, &checkout, &pack.skills, &[])?;
    if checkout.starts_with(&paths.packs) && checkout != paths.packs {
        std::fs::remove_dir_all(&checkout)
            .or_else(|error| match error.kind() {
                std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(error),
            })
            .map_err(|error| format!("Couldn't delete {}: {error}", checkout.display()))?;
    }
    save(paths, &packs)
}

// ---- background updates ----

/// Keeps auto-updating packs current: a first check shortly after launch,
/// then every six hours with jitter, backing off after failures.
#[derive(Default)]
pub struct Updater {
    cancel: tokio_util::sync::CancellationToken,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Updater {
    pub fn start(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let mut worker = self.worker.lock().unwrap();
        if worker.is_some() {
            return;
        }
        let cancel = self.cancel.clone();
        *worker = Some(tokio::spawn(async move {
            let mut delay = FIRST_CHECK;
            let mut retry = FIRST_RETRY;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let paths = Paths::current();
                let failed = tokio::select! {
                    _ = cancel.cancelled() => break,
                    failed = auto_update(&paths) => failed,
                };
                delay = if failed {
                    let current = retry;
                    retry = (retry * 2).min(CHECK_INTERVAL);
                    current
                } else {
                    retry = FIRST_RETRY;
                    CHECK_INTERVAL + Duration::from_secs(jitter_secs())
                };
            }
        }));
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let worker = self.worker.lock().unwrap().take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
    }
}

fn jitter_secs() -> u64 {
    (uuid::Uuid::new_v4().as_u128() % u128::from(MAX_JITTER_SECS)) as u64
}

/// Check and apply each auto-updating pack; whether any failed.
async fn auto_update(paths: &Paths) -> bool {
    let mut failed = false;
    for pack in load(paths).into_iter().filter(|pack| pack.auto_update) {
        let result = with_pack(paths, &pack.name, async |paths, pack| {
            check(paths, pack).await?;
            apply(paths, pack).await
        })
        .await;
        if let Err(error) = result {
            tracing::warn!(pack = %pack.name, %error, "skill pack update failed");
            failed = true;
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(temp: &Path) -> Paths {
        Paths::for_library(&temp.join(".zeron/skills"))
    }

    fn skill(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(
            dir.join(name).join(SKILL_FILE),
            format!("---\nname: {name}\ndescription: {name} skill\n---\nBody\n"),
        )
        .unwrap();
    }

    fn run(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    #[test]
    fn sources_become_clone_urls_and_names() {
        assert_eq!(
            parse_source("michael-denyer/pstack-claude").unwrap(),
            (
                "https://github.com/michael-denyer/pstack-claude".into(),
                "pstack-claude".into()
            )
        );
        assert_eq!(
            parse_source("https://gitlab.com/team/Skills.git/").unwrap(),
            ("https://gitlab.com/team/Skills.git".into(), "skills".into())
        );
        assert_eq!(
            parse_source("git@github.com:me/my.pack.git").unwrap().1,
            "my-pack"
        );
        for bad in [
            "",
            "/tmp/pack",
            "file:///tmp/pack",
            "ext::sh -c x",
            "-u/x",
            "a/b/c",
            "a b/c",
            "https://h/../x",
        ] {
            assert!(parse_source(bad).is_err(), "{bad} should be refused");
        }
        assert_eq!(
            parse_subpath(Some(" /packs/x/ ")).unwrap().as_deref(),
            Some("packs/x")
        );
        assert!(parse_subpath(Some("../x")).is_err());
        assert_eq!(parse_subpath(Some("")).unwrap(), None);
    }

    #[test]
    fn release_tags_skip_pre_releases() {
        assert!(is_release_tag("v0.9.86"));
        assert!(is_release_tag("2.1"));
        assert!(!is_release_tag("v1.0.0-rc.1"));
        assert!(!is_release_tag("nightly"));
        assert!(!is_release_tag("v"));
    }

    #[test]
    fn credential_config_keeps_order_scopes_and_resets() {
        let raw = b"credential.helper\nosxkeychain\0credential.helper\n\0credential.https://github.com.helper\n!gh auth git-credential\0credential.usehttppath\0";
        assert_eq!(
            parse_config_list(raw),
            [
                ("credential.helper".to_string(), Some("osxkeychain".to_string())),
                ("credential.helper".to_string(), Some(String::new())),
                (
                    "credential.https://github.com.helper".to_string(),
                    Some("!gh auth git-credential".to_string())
                ),
                ("credential.usehttppath".to_string(), None),
            ]
        );
    }

    #[test]
    fn releases_order_by_number_whatever_the_prefix() {
        assert!(release_key("2.0.0") > release_key("v1.9.0"));
        assert!(release_key("v1.10") > release_key("1.9.9"));
        assert_eq!(release_key("v1.2"), release_key("1.2.0"));
        assert_eq!(release_key("99999999999999999999999"), None);
    }

    #[test]
    fn layout_ignores_marketplace_paths_outside_the_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let outside = temp.path().join("outside");
        skill(&outside.join("skills"), "leak");
        std::fs::create_dir_all(repo.join(".claude-plugin")).unwrap();
        std::fs::write(
            repo.join(".claude-plugin/marketplace.json"),
            serde_json::json!({ "plugins": [{ "source": outside.to_string_lossy() }] }).to_string(),
        )
        .unwrap();
        assert!(detect(&repo, &repo).skills_roots.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn linked_manifests_and_folders_are_not_pack_skills() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        skill(&repo.join("skills"), "real");
        std::fs::create_dir_all(repo.join("skills/leak")).unwrap();
        let secret = temp.path().join("secret.md");
        std::fs::write(&secret, "---\nname: leak\n---\nnot yours\n").unwrap();
        std::os::unix::fs::symlink(&secret, repo.join("skills/leak/SKILL.md")).unwrap();
        skill(temp.path(), "elsewhere");
        std::os::unix::fs::symlink(temp.path().join("elsewhere"), repo.join("skills/elsewhere"))
            .unwrap();
        let layout = detect(&repo, &repo);
        let names: Vec<_> = pack_skills(&repo, &layout).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["real"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_relink_changes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(temp.path());
        let checkout = paths.checkout("pack");
        skill(&checkout.join("skills"), "old");
        skill(&checkout.join("skills"), "new");
        let layout = detect(&checkout, &checkout);
        let all = pack_skills(&checkout, &layout);
        let old: Vec<_> = all.iter().filter(|(n, _)| n == "old").cloned().collect();
        let new: Vec<_> = all.iter().filter(|(n, _)| n == "new").cloned().collect();
        relink(&paths, &checkout, &[], &old).unwrap();
        // A read-only library: removing `old` fails before `new` is linked.
        std::fs::set_permissions(&paths.skills, std::fs::Permissions::from_mode(0o555)).unwrap();
        assert!(relink(&paths, &checkout, &["old".into()], &new).is_err());
        std::fs::set_permissions(&paths.skills, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(links_into(&paths.skills.join("old"), &checkout));
        assert!(std::fs::symlink_metadata(paths.skills.join("new")).is_err());
    }

    #[test]
    fn detects_marketplace_plugin_and_flat_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        std::fs::create_dir_all(repo.join(".claude-plugin")).unwrap();
        std::fs::write(
            repo.join(".claude-plugin/marketplace.json"),
            r#"{"plugins":[{"name":"p","source":"./plugins/p"},{"source":"../escape"}]}"#,
        )
        .unwrap();
        skill(&repo.join("plugins/p/skills"), "how");
        std::fs::create_dir_all(repo.join("plugins/p/.claude-plugin")).unwrap();
        std::fs::write(
            repo.join("plugins/p/.claude-plugin/plugin.json"),
            r#"{"version":"1.2.0"}"#,
        )
        .unwrap();
        let layout = detect(repo, repo);
        assert_eq!(layout.skills_roots, ["plugins/p/skills"]);
        assert_eq!(layout.plugin_dirs, ["plugins/p"]);
        assert_eq!(
            declared_version(repo, repo, &layout).as_deref(),
            Some("1.2.0")
        );
        std::fs::write(repo.join("VERSION"), "1.3.0\n").unwrap();
        assert_eq!(
            declared_version(repo, repo, &layout).as_deref(),
            Some("1.3.0")
        );

        let flat = temp.path().join("flat");
        skill(&flat.join("skills"), "tdd");
        assert_eq!(detect(&flat, &flat).skills_roots, ["skills"]);
        let bare = temp.path().join("bare");
        skill(&bare, "why");
        assert_eq!(detect(&bare, &bare).skills_roots, ["."]);
    }

    #[test]
    fn changelog_entries_end_at_the_next_heading() {
        let text = "# CHANGES\n\nIntro.\n\n## 0.9.86 - sync\n\nNew things.\n\n### Notes\n\nDetail.\n\n## 0.9.85 - older\n\nOld.\n";
        assert_eq!(
            changes_entry(text, "v0.9.86").as_deref(),
            Some("New things.\n\n### Notes\n\nDetail.")
        );
        assert_eq!(changes_entry(text, "0.9.85").as_deref(), Some("Old."));
        // A version that only prefixes another must not match it.
        assert_eq!(changes_entry(text, "0.9.8"), None);
        assert_eq!(
            changes_entry("## [1.2.0] - 2026-01-01\n- fix\n", "1.2.0").as_deref(),
            Some("- fix")
        );
    }

    #[cfg(unix)]
    #[test]
    fn relinking_adds_removes_and_never_touches_other_skills() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(temp.path());
        let checkout = paths.checkout("pack");
        skill(&checkout.join("skills"), "how");
        skill(&checkout.join("skills"), "tdd");
        skill(&checkout.join("skills"), "why");
        skill(&paths.skills, "tdd"); // the user's own
        let layout = detect(&checkout, &checkout);
        let all = pack_skills(&checkout, &layout);
        assert_eq!(clashes(&paths, &all), ["tdd"]);
        let (linked, skipped) = relink(&paths, &checkout, &[], &all).unwrap();
        assert_eq!(
            (linked.clone(), skipped),
            (vec!["how".into(), "why".into()], vec!["tdd".to_string()])
        );
        assert!(links_into(&paths.skills.join("how"), &checkout));
        assert!(!links_into(&paths.skills.join("tdd"), &checkout));
        assert_eq!(
            owner_of_link(&paths, &paths.skills.join("how")).as_deref(),
            Some("pack")
        );
        // `why` leaves the pack: its link goes, `how` stays, the user's `tdd` stays.
        std::fs::remove_dir_all(checkout.join("skills/why")).unwrap();
        let now = pack_skills(&checkout, &layout);
        let (linked, _) = relink(&paths, &checkout, &linked, &now).unwrap();
        assert_eq!(linked, ["how"]);
        assert!(std::fs::symlink_metadata(paths.skills.join("why")).is_err());
        assert!(paths.skills.join("tdd/SKILL.md").is_file());
        relink(&paths, &checkout, &linked, &[]).unwrap();
        assert!(std::fs::symlink_metadata(paths.skills.join("how")).is_err());
    }

    /// A local repository standing in for a pack host: install it, publish a
    /// release with a new skill and a dropped one, update, then remove.
    #[cfg(unix)]
    #[tokio::test]
    async fn installs_updates_and_removes_a_tagged_pack() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(temp.path());
        let origin = temp.path().join("origin");
        skill(&origin.join("skills"), "how");
        skill(&origin.join("skills"), "old");
        std::fs::write(origin.join("VERSION"), "1.0.0\n").unwrap();
        std::fs::write(
            origin.join("CHANGES.md"),
            "# Changes\n\n## 1.1.0 - more\n\nAdds new.\n\n## 1.0.0\n\nFirst.\n",
        )
        .unwrap();
        run(&origin, &["init", "-q", "-b", "main"]);
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "one"]);
        run(&origin, &["tag", "v1.0.0"]);

        // Local transports are refused for real sources; stage the clone by
        // hand the way `inspect_at` leaves it, then install through it.
        let token = uuid::Uuid::new_v4().to_string();
        let stage = paths.staging.join(&token);
        std::fs::create_dir_all(&stage).unwrap();
        run(
            temp.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                stage.join("checkout").to_str().unwrap(),
            ],
        );
        let checkout = stage.join("checkout");
        let commit = commit_of(&checkout, "v1.0.0").await.unwrap();
        checkout_detached(&checkout, &commit).await.unwrap();
        let layout = detect(&checkout, &checkout);
        let inspection = PackInspection {
            token: token.clone(),
            name: "pack".into(),
            source: "me/pack".into(),
            url: origin.to_string_lossy().into_owned(),
            subpath: None,
            channel: Channel::Tags,
            reference: "v1.0.0".into(),
            commit,
            version: declared_version(&checkout, &checkout, &layout),
            skills_roots: layout.skills_roots.clone(),
            plugin_dirs: layout.plugin_dirs.clone(),
            skills: vec!["how".into(), "old".into()],
            clashes: vec![],
        };
        std::fs::write(
            stage.join(INSPECTION_FILE),
            serde_json::to_vec(&inspection).unwrap(),
        )
        .unwrap();
        let pack = install_at(&paths, &token, true).await.unwrap();
        assert_eq!(pack.skills, ["how", "old"]);
        assert_eq!(pack.version.as_deref(), Some("1.0.0"));
        assert!(!stage.exists());
        assert!(paths.skills.join("how/SKILL.md").is_file());

        // Publish 1.1.0: `new` arrives, `old` leaves.
        std::fs::remove_dir_all(origin.join("skills/old")).unwrap();
        skill(&origin.join("skills"), "new");
        std::fs::write(origin.join("VERSION"), "1.1.0\n").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "two"]);
        run(&origin, &["tag", "v1.1.0"]);

        // `git` refuses file transports by design; fetch with a plain git
        // for the test, then let the update apply what it finds.
        run(
            &paths.checkout("pack"),
            &["fetch", "-q", "--tags", "origin"],
        );
        let mut record = load(&paths).remove(0);
        record.update = Some(PackUpdate {
            reference: "v1.1.0".into(),
            commit: commit_of(&paths.checkout("pack"), "v1.1.0").await.unwrap(),
            version: Some("1.1.0".into()),
            changes: changes_at(&paths.checkout("pack"), "v1.1.0", &record, "1.1.0").await,
        });
        assert_eq!(
            record.update.as_ref().unwrap().changes.as_deref(),
            Some("Adds new.")
        );
        save(&paths, &[record]).unwrap();
        // An update that can't be recorded is undone: release and links.
        {
            use std::os::unix::fs::PermissionsExt;
            let old_commit = commit_of(&paths.checkout("pack"), "HEAD").await.unwrap();
            std::fs::set_permissions(&paths.packs, std::fs::Permissions::from_mode(0o555)).unwrap();
            let failed = update_at(&paths, "pack").await;
            std::fs::set_permissions(&paths.packs, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(failed.is_err());
            assert_eq!(commit_of(&paths.checkout("pack"), "HEAD").await.unwrap(), old_commit);
            assert!(paths.skills.join("old/SKILL.md").is_file());
            assert!(std::fs::symlink_metadata(paths.skills.join("new")).is_err());
            assert_eq!(load(&paths)[0].version.as_deref(), Some("1.0.0"));
        }
        let updated = update_at(&paths, "pack").await.unwrap();
        assert_eq!(updated.skills, ["how", "new"]);
        assert_eq!(updated.version.as_deref(), Some("1.1.0"));
        assert_eq!(updated.reference, "v1.1.0");
        assert!(updated.update.is_none());
        assert!(std::fs::symlink_metadata(paths.skills.join("old")).is_err());
        assert!(paths.skills.join("new/SKILL.md").is_file());

        remove_at(&paths, "pack").await.unwrap();
        assert!(load(&paths).is_empty());
        assert!(!paths.checkout("pack").exists());
        assert!(std::fs::symlink_metadata(paths.skills.join("how")).is_err());
    }

    /// Live: an earlier Zeron's partial clone becomes a full one, with no
    /// object left to fetch lazily.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_partial_clones_become_full() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("pack");
        run(
            temp.path(),
            &[
                "clone",
                "-q",
                "--filter=blob:none",
                "--no-checkout",
                "https://github.com/michael-denyer/pstack-claude",
                checkout.to_str().unwrap(),
            ],
        );
        // An interrupted migration: the filter is gone but objects are still
        // missing. The promisor markers remain, so it is picked up again.
        run(&checkout, &["config", "--unset-all", "remote.origin.partialclonefilter"]);
        ensure_full_clone(&checkout).await.unwrap();
        assert!(git(Some(&checkout), &["config", "--get", "remote.origin.partialclonefilter"]).await.is_err());
        assert!(git(Some(&checkout), &["config", "--get", "remote.origin.promisor"]).await.is_err());
        let missing = git(
            Some(&checkout),
            &["rev-list", "--objects", "--all", "--missing=print"],
        )
        .await
        .unwrap();
        assert!(!missing.lines().any(|line| line.starts_with('?')));
        // Already full: nothing to do.
        ensure_full_clone(&checkout).await.unwrap();
    }

    /// A filter that can't be removed stops the migration instead of letting
    /// a filtered refetch pass for a full one.
    #[tokio::test]
    async fn a_stuck_filter_stops_the_migration() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run(&repo, &["init", "-q"]);
        run(&repo, &["config", "remote.origin.partialclonefilter", "blob:none"]);
        std::fs::write(repo.join(".git/config.lock"), "").unwrap();
        assert!(ensure_full_clone(&repo).await.is_err());
    }

    /// Live: `cargo test -p zeron-engine skill_packs::tests::live -- --ignored`.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_inspect_and_install_pstack() {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(temp.path());
        skill(&paths.skills, "how"); // a taken name
        let inspection = inspect_at(&paths, "michael-denyer/pstack-claude", None)
            .await
            .unwrap();
        assert_eq!(inspection.name, "pstack-claude");
        assert_eq!(inspection.channel, Channel::Tags);
        assert_eq!(inspection.skills_roots, ["plugins/pstack/skills"]);
        assert_eq!(inspection.plugin_dirs, ["plugins/pstack"]);
        assert!(inspection.skills.len() > 40, "{:?}", inspection.skills);
        assert_eq!(inspection.clashes, ["how"]);
        let pack = install_at(&paths, &inspection.token, false).await.unwrap();
        assert_eq!(pack.skipped, ["how"]);
        assert!(paths.skills.join("poteto-mode/SKILL.md").is_file());
        // Skills reference siblings by relative path through the link.
        assert!(paths.skills.join("why/../poteto-mode/SKILL.md").is_file());
        let mut checked = load(&paths).remove(0);
        check(&paths, &mut checked).await.unwrap();
        assert!(checked.update.is_none(), "the newest tag is installed");
        println!(
            "{} {:?} skills={}",
            pack.reference,
            pack.version,
            pack.skills.len()
        );
    }
}
