//! The shared skill library: the skills in `~/.zeron/skills` that every
//! harness loads, and installs into it from the public skills.sh registry.
//!
//! The engine owns every path check. Callers name a skill folder by the path
//! the listing returned; anything outside the shared root is refused here,
//! not hidden by the UI. Registry installs download the skill's folder from
//! GitHub (the source skills.sh indexes), stage it beside the library, and
//! move it in only once its `SKILL.md` parses.
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

const SKILLS_SH: &str = "https://skills.sh";
const SKILLS_SH_HOME: &str = "https://www.skills.sh/";
const GITHUB_API: &str = "https://api.github.com";
const GITHUB_RAW: &str = "https://raw.githubusercontent.com";
/// Written into a registry install so the library can show where it came from.
const PROVENANCE_FILE: &str = ".zeron-registry.json";
const SKILL_FILE: &str = "SKILL.md";

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const POPULAR_TTL: Duration = Duration::from_secs(30 * 60);
const PREVIEW_TTL: Duration = Duration::from_secs(30 * 60);
const SEARCH_LIMIT: usize = 50;
/// `SKILL.md`s read to match a skill by its declared name.
const MAX_NAME_PROBES: usize = 80;
const POPULAR_LIMIT: usize = 60;

/// Install bounds: a skill is a handful of instruction and helper files.
const MAX_INSTALL_FILES: usize = 200;
const MAX_INSTALL_FILE_BYTES: u64 = 1024 * 1024;
const MAX_INSTALL_BYTES: u64 = 5 * 1024 * 1024;
/// Library walk bounds, so a stray checkout under the root can't stall it.
const MAX_LIBRARY_DEPTH: usize = 3;
const MAX_LIBRARY_ENTRIES: usize = 2048;
const MAX_DETAIL_FILES: usize = 200;
const MAX_SKILL_TEXT: u64 = 256 * 1024;

/// One skill in the shared library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySkill {
    pub name: String,
    pub description: String,
    /// The skill's folder: what Remove deletes and Reveal shows.
    pub directory: String,
    /// Its `SKILL.md`, the file Open edits.
    pub file: String,
    /// The skills.sh id (`owner/repo/skill`) when it was installed from there.
    pub registry_id: Option<String>,
    pub modified_at: Option<DateTime<Utc>>,
    /// The folder is a link to a skill kept elsewhere; Remove unlinks it.
    #[serde(default)]
    pub linked: bool,
    /// The skill pack this skill is linked from. Pack skills come and go
    /// with their pack.
    #[serde(default)]
    pub pack: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    /// The shared root, whether or not it exists yet.
    pub root: String,
    pub skills: Vec<LibrarySkill>,
    #[serde(default)]
    pub packs: Vec<crate::skill_packs::SkillPack>,
}

/// A library skill's instructions and the files it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillContents {
    /// The `SKILL.md` body, frontmatter removed.
    pub markdown: String,
    /// Paths relative to the skill folder, `SKILL.md` first.
    pub files: Vec<String>,
    pub truncated: bool,
}

/// One skills.sh listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySkill {
    /// `owner/repo/skill`.
    pub id: String,
    /// `owner/repo`.
    pub source: String,
    pub skill_id: String,
    pub name: String,
    pub installs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySearch {
    pub query: String,
    /// The query was too short to search, so these are skills.sh's most
    /// installed skills instead.
    pub popular: bool,
    pub skills: Vec<RegistrySkill>,
}

/// A registry skill's `SKILL.md`, read before installing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryPreview {
    pub id: String,
    pub description: Option<String>,
    pub markdown: String,
}

pub fn root() -> PathBuf {
    zeron_harness::shared_skills_root()
}

// ---- the library ----

pub async fn list() -> Result<Library, String> {
    let root = root();
    tokio::task::spawn_blocking(move || list_at(&root))
        .await
        .map_err(|error| error.to_string())?
}

fn list_at(root: &Path) -> Result<Library, String> {
    let mut skills = Vec::new();
    let mut remaining = MAX_LIBRARY_ENTRIES;
    let mut seen = HashSet::new();
    walk_library(root, 0, &mut remaining, &mut seen, &mut skills);
    let packs = crate::skill_packs::Paths::for_library(root);
    for skill in &mut skills {
        if skill.linked {
            skill.pack = crate::skill_packs::owner_of_link(&packs, Path::new(&skill.directory));
        }
    }
    skills.sort_by(|left: &LibrarySkill, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.directory.cmp(&right.directory))
    });
    Ok(Library {
        root: root.to_string_lossy().into_owned(),
        skills,
        packs: crate::skill_packs::load(&packs),
    })
}

/// A folder holding a `SKILL.md` is one skill; anything else may group skills
/// one level further down, as the harness scan allows.
fn walk_library(
    dir: &Path,
    depth: usize,
    remaining: &mut usize,
    seen: &mut HashSet<PathBuf>,
    skills: &mut Vec<LibrarySkill>,
) {
    if depth > MAX_LIBRARY_DEPTH || *remaining == 0 {
        return;
    }
    let Ok(canonical) = std::fs::canonicalize(dir) else {
        return;
    };
    if !seen.insert(canonical) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries
        .take(*remaining)
        .inspect(|_| *remaining -= 1)
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    children.sort();
    for child in children {
        match library_skill(&child) {
            Some(skill) => skills.push(skill),
            None => walk_library(&child, depth + 1, remaining, seen, skills),
        }
    }
}

fn library_skill(directory: &Path) -> Option<LibrarySkill> {
    let file = directory.join(SKILL_FILE);
    let manifest = zeron_harness::read_skill_manifest(&file)?;
    let modified_at = std::fs::metadata(&file)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(DateTime::<Utc>::from);
    Some(LibrarySkill {
        name: manifest.name,
        description: manifest.description,
        directory: directory.to_string_lossy().into_owned(),
        file: file.to_string_lossy().into_owned(),
        registry_id: read_provenance(directory),
        modified_at,
        linked: std::fs::symlink_metadata(directory)
            .is_ok_and(|metadata| metadata.file_type().is_symlink()),
        pack: None,
    })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provenance {
    registry: String,
    id: String,
    installed_at: DateTime<Utc>,
}

fn read_provenance(directory: &Path) -> Option<String> {
    let text = std::fs::read_to_string(directory.join(PROVENANCE_FILE)).ok()?;
    let provenance: Provenance = serde_json::from_str(&text).ok()?;
    (provenance.registry == "skills.sh" && parse_registry_id(&provenance.id).is_ok())
        .then_some(provenance.id)
}

/// `path` named as a skill folder strictly inside the shared root: relative
/// components only, and no `..` — symlinks are judged by the caller.
fn lexically_inside(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "That skill isn't in the shared skills folder.".to_string())?;
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("That skill isn't in the shared skills folder.".into());
    }
    Ok(())
}

pub async fn contents(directory: String) -> Result<SkillContents, String> {
    let root = root();
    tokio::task::spawn_blocking(move || contents_at(&root, Path::new(&directory)))
        .await
        .map_err(|error| error.to_string())?
}

fn contents_at(root: &Path, directory: &Path) -> Result<SkillContents, String> {
    lexically_inside(root, directory)?;
    let file = directory.join(SKILL_FILE);
    let text =
        read_bounded(&file).map_err(|_| "This skill's SKILL.md can't be read.".to_string())?;
    let (_, body) = zeron_harness::skill_document(&text);
    let mut files = Vec::new();
    let mut truncated = false;
    collect_files(directory, directory, 0, &mut files, &mut truncated);
    files.sort_by(|left: &String, right| {
        (left != SKILL_FILE)
            .cmp(&(right != SKILL_FILE))
            .then_with(|| left.cmp(right))
    });
    Ok(SkillContents {
        markdown: body.to_owned(),
        files,
        truncated,
    })
}

fn read_bounded(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut text = String::new();
    file.take(MAX_SKILL_TEXT).read_to_string(&mut text)?;
    Ok(text)
}

fn collect_files(
    base: &Path,
    dir: &Path,
    depth: usize,
    files: &mut Vec<String>,
    truncated: &mut bool,
) {
    if depth > 6 {
        *truncated = true;
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= MAX_DETAIL_FILES {
            *truncated = true;
            return;
        }
        let name = entry.file_name();
        if name == PROVENANCE_FILE || name == ".DS_Store" {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_files(base, &path, depth + 1, files, truncated);
        } else if let Ok(relative) = path.strip_prefix(base) {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

pub async fn remove(directory: String) -> Result<Library, String> {
    let root = root();
    tokio::task::spawn_blocking(move || {
        remove_at(&root, Path::new(&directory))?;
        list_at(&root)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn remove_at(root: &Path, directory: &Path) -> Result<(), String> {
    lexically_inside(root, directory)?;
    let canonical_root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
    let metadata = std::fs::symlink_metadata(directory)
        .map_err(|_| "That skill is already gone.".to_string())?;
    // A linked skill lives elsewhere: unlink it from the library and leave
    // its target alone. The link itself must sit inside the root.
    if metadata.file_type().is_symlink() {
        let packs = crate::skill_packs::Paths::for_library(root);
        if let Some(pack) = crate::skill_packs::owner_of_link(&packs, directory) {
            return Err(format!(
                "This skill comes from the {pack} pack, and its next update would bring it back. Remove the pack instead."
            ));
        }
        let parent = directory
            .parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .ok_or_else(|| "That skill's folder can't be read.".to_string())?;
        if !parent.starts_with(&canonical_root) {
            return Err("That skill isn't in the shared skills folder.".into());
        }
        return remove_link(directory).map_err(|error| format!("Couldn't remove it: {error}"));
    }
    let canonical = std::fs::canonicalize(directory).map_err(|error| error.to_string())?;
    if canonical == canonical_root || !canonical.starts_with(&canonical_root) {
        return Err("That skill isn't in the shared skills folder.".into());
    }
    if !canonical.join(SKILL_FILE).is_file() {
        return Err("That folder isn't a skill.".into());
    }
    std::fs::remove_dir_all(&canonical).map_err(|error| format!("Couldn't remove it: {error}"))
}

fn remove_link(path: &Path) -> std::io::Result<()> {
    // Windows directory links are removed as directories; this never
    // follows the link into its target.
    std::fs::remove_file(path).or_else(|error| {
        if cfg!(windows) {
            std::fs::remove_dir(path)
        } else {
            Err(error)
        }
    })
}

// ---- the registry ----

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("zeron/", env!("CARGO_PKG_VERSION")))
            .timeout(FETCH_TIMEOUT)
            .build()
            .unwrap_or_default()
    })
}

fn unreachable(_: reqwest::Error) -> String {
    "skills.sh couldn't be reached. Check your connection and try again.".into()
}

pub async fn search(query: String) -> Result<RegistrySearch, String> {
    let trimmed = query.trim().to_owned();
    // skills.sh refuses queries under two characters; show what's popular.
    if trimmed.chars().count() < 2 {
        return Ok(RegistrySearch {
            query,
            popular: true,
            skills: popular().await?,
        });
    }
    #[derive(Deserialize)]
    struct Response {
        #[serde(default)]
        skills: Vec<ApiSkill>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ApiSkill {
        source: String,
        skill_id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        installs: u64,
    }
    let url = reqwest::Url::parse_with_params(
        &format!("{SKILLS_SH}/api/search"),
        &[
            ("q", trimmed.as_str()),
            ("limit", &SEARCH_LIMIT.to_string()),
        ],
    )
    .map_err(|error| error.to_string())?;
    let response: Response = client()
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(unreachable)?
        .json()
        .await
        .map_err(|_| "skills.sh sent a response Zeron doesn't understand.".to_string())?;
    let skills = response
        .skills
        .into_iter()
        .filter_map(|skill| {
            registry_skill(
                skill.source,
                skill.skill_id.clone(),
                skill.name.unwrap_or(skill.skill_id),
                skill.installs,
            )
        })
        .collect();
    Ok(RegistrySearch {
        query,
        popular: false,
        skills,
    })
}

fn registry_skill(
    source: String,
    skill_id: String,
    name: String,
    installs: u64,
) -> Option<RegistrySkill> {
    let id = format!("{source}/{skill_id}");
    parse_registry_id(&id).ok()?;
    Some(RegistrySkill {
        id,
        source,
        skill_id,
        name,
        installs,
    })
}

async fn popular() -> Result<Vec<RegistrySkill>, String> {
    static POPULAR: Mutex<Option<(Instant, Vec<RegistrySkill>)>> = Mutex::new(None);
    if let Some((at, skills)) = POPULAR.lock().unwrap().as_ref()
        && at.elapsed() < POPULAR_TTL
    {
        return Ok(skills.clone());
    }
    let html = client()
        .get(SKILLS_SH_HOME)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(unreachable)?
        .text()
        .await
        .map_err(unreachable)?;
    let mut skills = parse_directory(&html);
    if skills.is_empty() {
        return Err("skills.sh sent a response Zeron doesn't understand.".into());
    }
    skills.truncate(POPULAR_LIMIT);
    *POPULAR.lock().unwrap() = Some((Instant::now(), skills.clone()));
    Ok(skills)
}

/// The home page embeds its leaderboard as escaped JSON in the page's
/// streamed payload: `\"source\":\"…\",\"skillId\":\"…\",\"name\":\"…\",
/// \"installs\":N`. Read those records in order, first occurrence wins.
fn parse_directory(html: &str) -> Vec<RegistrySkill> {
    const SOURCE: &str = r#"\"source\":\""#;
    const SKILL_ID: &str = r#"\",\"skillId\":\""#;
    const NAME: &str = r#"\",\"name\":\""#;
    const INSTALLS: &str = r#"\",\"installs\":"#;
    fn field<'a>(rest: &'a str, next: &str) -> Option<(&'a str, &'a str)> {
        let end = rest.find(next)?;
        let value = &rest[..end];
        (!value.contains(['"', '\\'])).then_some((value, &rest[end + next.len()..]))
    }
    let mut skills = Vec::new();
    let mut seen = HashSet::new();
    let mut rest = html;
    while let Some(at) = rest.find(SOURCE) {
        rest = &rest[at + SOURCE.len()..];
        let record = (|| {
            let (source, tail) = field(rest, SKILL_ID)?;
            let (skill_id, tail) = field(tail, NAME)?;
            let (name, tail) = field(tail, INSTALLS)?;
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            let installs = tail[..digits].parse().ok()?;
            Some((source, skill_id, name, installs))
        })();
        let Some((source, skill_id, name, installs)) = record else {
            continue;
        };
        if let Some(skill) = registry_skill(source.into(), skill_id.into(), name.into(), installs)
            && seen.insert(skill.id.clone())
        {
            skills.push(skill);
        }
    }
    skills
}

/// A GitHub-backed registry id: `owner/repo/skill`, with an optional
/// `github.com/` prefix. Other hosts (and anything path-like) are refused.
fn parse_registry_id(id: &str) -> Result<(&str, &str, &str), String> {
    let invalid = || format!("“{id}” isn't a skills.sh skill Zeron can install.");
    let id = id.strip_prefix("github.com/").unwrap_or(id);
    let mut parts = id.split('/');
    let (Some(owner), Some(repo), Some(skill), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    let segment = |value: &str, dots: bool| {
        !value.is_empty()
            && value.len() <= 100
            && value != "."
            && value != ".."
            && !value.starts_with(['.', '-'])
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_') || (dots && c == '.'))
    };
    // A dotted owner is a domain, not a GitHub account.
    if !segment(owner, false) || !segment(repo, true) || !segment(skill, true) || skill.len() > 64 {
        return Err(invalid());
    }
    Ok((owner, repo, skill))
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        (byte as char).to_string()
                    }
                    _ => format!("%{byte:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// `path` at `revision` (a commit SHA, or `HEAD` for previews).
async fn fetch_raw(
    owner: &str,
    repo: &str,
    revision: &str,
    path: &str,
) -> Result<Option<Vec<u8>>, String> {
    let url = format!(
        "{GITHUB_RAW}/{owner}/{repo}/{revision}/{}",
        encode_path(path)
    );
    let response = client().get(url).send().await.map_err(|_| {
        "GitHub couldn't be reached. Check your connection and try again.".to_string()
    })?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|error| format!("GitHub refused the download ({error})."))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_INSTALL_FILE_BYTES)
    {
        return Err(format!("{path} is too large to install."));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "The download was interrupted. Try again.".to_string())?;
    if bytes.len() as u64 > MAX_INSTALL_FILE_BYTES {
        return Err(format!("{path} is too large to install."));
    }
    Ok(Some(bytes.to_vec()))
}

#[derive(Debug, Clone, Deserialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
}

/// The repository at one commit: everything an install reads comes from it,
/// so a push mid-install can't mix files from two revisions.
struct Snapshot {
    commit: String,
    tree: Vec<TreeEntry>,
}

async fn github_get(owner: &str, repo: &str, url: String, accept: &str) -> Result<reqwest::Response, String> {
    let response = client()
        .get(url)
        .header("accept", accept)
        .send()
        .await
        .map_err(|_| {
            "GitHub couldn't be reached. Check your connection and try again.".to_string()
        })?;
    match response.status() {
        status if status.is_success() => Ok(response),
        reqwest::StatusCode::NOT_FOUND => Err(format!("{owner}/{repo} wasn't found on GitHub.")),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::TOO_MANY_REQUESTS => {
            Err("GitHub's download limit was reached. Try again in a few minutes.".into())
        }
        status => Err(format!("GitHub refused the download ({status}).")),
    }
}

async fn repository_snapshot(owner: &str, repo: &str) -> Result<Snapshot, String> {
    #[derive(Deserialize)]
    struct Tree {
        tree: Vec<TreeEntry>,
    }
    let unreadable = || "GitHub sent a response Zeron doesn't understand.".to_string();
    let commit = github_get(
        owner,
        repo,
        format!("{GITHUB_API}/repos/{owner}/{repo}/commits/HEAD"),
        "application/vnd.github.sha",
    )
    .await?
    .text()
    .await
    .map_err(|_| unreadable())?
    .trim()
    .to_owned();
    if commit.len() < 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unreadable());
    }
    let tree: Tree = github_get(
        owner,
        repo,
        format!("{GITHUB_API}/repos/{owner}/{repo}/git/trees/{commit}?recursive=1"),
        "application/vnd.github+json",
    )
    .await?
    .json()
    .await
    .map_err(|_| unreadable())?;
    Ok(Snapshot {
        commit,
        tree: tree.tree,
    })
}

/// The folder in `tree` that holds `skill`'s `SKILL.md`: the shallowest one
/// named after the skill, else the repository root when it is one skill.
fn skill_folder(tree: &[TreeEntry], skill: &str) -> Option<String> {
    let mut candidates: Vec<&str> = tree
        .iter()
        .filter(|entry| entry.kind == "blob")
        .filter_map(|entry| entry.path.strip_suffix("/SKILL.md"))
        .filter(|folder| {
            folder
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case(skill))
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.matches('/')
            .count()
            .cmp(&right.matches('/').count())
            .then_with(|| left.cmp(right))
    });
    if let Some(folder) = candidates.first() {
        return Some((*folder).to_owned());
    }
    tree.iter()
        .any(|entry| entry.kind == "blob" && entry.path == SKILL_FILE)
        .then(String::new)
}

/// Where `skill` lives in `tree`: a folder named after it, else the folder
/// whose `SKILL.md` declares that name (folders are often named differently,
/// like `skills/react-best-practices` for `vercel-react-best-practices`).
async fn locate_skill(
    owner: &str,
    repo: &str,
    skill: &str,
    snapshot: &Snapshot,
) -> Result<String, String> {
    if let Some(folder) = skill_folder(&snapshot.tree, skill) {
        return Ok(folder);
    }
    let candidates: Vec<String> = snapshot
        .tree
        .iter()
        .filter(|entry| entry.kind == "blob")
        .filter(|entry| entry.path.ends_with("/SKILL.md"))
        .take(MAX_NAME_PROBES)
        .map(|entry| entry.path.clone())
        .collect();
    let probes: Vec<_> = candidates
        .into_iter()
        .map(|path| {
            let (owner, repo) = (owner.to_owned(), repo.to_owned());
            let commit = snapshot.commit.clone();
            async move {
                let text = fetch_raw(&owner, &repo, &commit, &path).await.ok().flatten()?;
                let name = zeron_harness::skill_declared_name(&String::from_utf8_lossy(&text))?;
                Some((path, name))
            }
        })
        .collect();
    let mut found: Vec<String> = futures::stream::iter(probes)
        .buffer_unordered(8)
        .filter_map(|probe| async move { probe })
        .filter(|(_, name)| futures::future::ready(name.eq_ignore_ascii_case(skill)))
        .map(|(path, _)| path.trim_end_matches("/SKILL.md").to_owned())
        .collect()
        .await;
    found.sort_by_key(|folder| (folder.matches('/').count(), folder.clone()));
    found
        .into_iter()
        .next()
        .ok_or_else(|| format!("{skill} wasn't found in {owner}/{repo}."))
}

/// The places skill repositories conventionally keep a skill, tried before
/// listing the whole repository.
fn conventional_folders(skill: &str) -> [String; 5] {
    [
        format!("skills/{skill}"),
        skill.to_owned(),
        format!(".claude/skills/{skill}"),
        format!(".agents/skills/{skill}"),
        format!(".github/skills/{skill}"),
    ]
}

pub async fn preview(id: String) -> Result<RegistryPreview, String> {
    static PREVIEWS: OnceLock<Mutex<HashMap<String, (Instant, RegistryPreview)>>> = OnceLock::new();
    let cache = PREVIEWS.get_or_init(Default::default);
    if let Some((at, preview)) = cache.lock().unwrap().get(&id)
        && at.elapsed() < PREVIEW_TTL
    {
        return Ok(preview.clone());
    }
    let (owner, repo, skill) = parse_registry_id(&id)?;
    let attempts = conventional_folders(skill).map(|folder| {
        let (owner, repo) = (owner.to_owned(), repo.to_owned());
        let path = format!("{folder}/{SKILL_FILE}");
        async move { fetch_raw(&owner, &repo, "HEAD", &path).await }
    });
    let mut text = futures::future::join_all(attempts)
        .await
        .into_iter()
        .find_map(|result| result.ok().flatten());
    if text.is_none() {
        let snapshot = repository_snapshot(owner, repo).await?;
        let folder = locate_skill(owner, repo, skill, &snapshot).await?;
        let path = if folder.is_empty() {
            SKILL_FILE.to_owned()
        } else {
            format!("{folder}/{SKILL_FILE}")
        };
        text = fetch_raw(owner, repo, &snapshot.commit, &path).await?;
    }
    let text = text.ok_or_else(|| format!("{skill} wasn't found in {owner}/{repo}."))?;
    let text = String::from_utf8_lossy(&text);
    let (description, body) = zeron_harness::skill_document(&text);
    let preview = RegistryPreview {
        id: id.clone(),
        description,
        markdown: body.to_owned(),
    };
    cache
        .lock()
        .unwrap()
        .insert(id, (Instant::now(), preview.clone()));
    Ok(preview)
}

/// Every file of the skill at `folder`, as `(relative path, executable)`.
fn install_plan(tree: &[TreeEntry], folder: &str) -> Result<Vec<(String, bool)>, String> {
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in tree {
        let Some(relative) = entry.path.strip_prefix(&prefix) else {
            continue;
        };
        // Regular files only: links and submodules could point anywhere.
        let executable = match (entry.kind.as_str(), entry.mode.as_str()) {
            ("blob", "100644") => false,
            ("blob", "100755") => true,
            _ => continue,
        };
        if !safe_relative(relative) {
            return Err("This skill contains a file path Zeron won't write.".into());
        }
        let size = entry.size.unwrap_or(0);
        total += size;
        if size > MAX_INSTALL_FILE_BYTES {
            return Err(format!("{relative} is too large to install."));
        }
        files.push((relative.to_owned(), executable));
        if files.len() > MAX_INSTALL_FILES || total > MAX_INSTALL_BYTES {
            return Err("This skill is too large to install.".into());
        }
    }
    if !files.iter().any(|(path, _)| path == SKILL_FILE) {
        return Err("This skill has no SKILL.md.".into());
    }
    Ok(files)
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

pub async fn install(id: String) -> Result<LibrarySkill, String> {
    let root = root();
    install_into(&root, &id).await
}

async fn install_into(root: &Path, id: &str) -> Result<LibrarySkill, String> {
    let (owner, repo, skill) = parse_registry_id(id)?;
    let target = root.join(skill);
    if tokio::fs::symlink_metadata(&target).await.is_ok() {
        return Err(format!("A skill named {skill} is already installed."));
    }
    let snapshot = repository_snapshot(owner, repo).await?;
    let folder = locate_skill(owner, repo, skill, &snapshot).await?;
    let files = install_plan(&snapshot.tree, &folder)?;
    // The tree's sizes are a plan; the bytes actually downloaded are bounded too.
    let downloaded = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    // Stage beside the library, never inside it: harnesses scan the root,
    // and a half-written skill must not be picked up.
    let staging_parent = root
        .parent()
        .map(|parent| parent.join(".skills-staging"))
        .ok_or_else(|| "The shared skills folder has no parent.".to_string())?;
    let staging = staging_parent.join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|error| format!("Couldn't prepare the install: {error}"))?;
    let result = async {
        let source = |path: &str| {
            if folder.is_empty() {
                path.to_owned()
            } else {
                format!("{folder}/{path}")
            }
        };
        let downloads: Vec<_> = files
            .iter()
            .map(|(path, executable)| {
                let (owner, repo) = (owner.to_owned(), repo.to_owned());
                let (path, executable) = (path.clone(), *executable);
                let commit = snapshot.commit.clone();
                let downloaded = downloaded.clone();
                let remote = source(&path);
                let local = staging.join(&path);
                async move {
                    let bytes = fetch_raw(&owner, &repo, &commit, &remote)
                        .await?
                        .ok_or_else(|| format!("{path} disappeared from {owner}/{repo}."))?;
                    let total = downloaded
                        .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed)
                        + bytes.len() as u64;
                    if total > MAX_INSTALL_BYTES {
                        return Err("This skill is too large to install.".to_string());
                    }
                    write_file(&local, &bytes, executable)
                        .await
                        .map_err(|error| format!("Couldn't save {path}: {error}"))
                }
            })
            .collect();
        futures::stream::iter(downloads)
            .buffer_unordered(8)
            .try_collect::<Vec<()>>()
            .await?;
        let manifest = zeron_harness::read_skill_manifest(&staging.join(SKILL_FILE))
            .ok_or_else(|| "This skill's SKILL.md isn't valid.".to_string())?;
        let provenance = Provenance {
            registry: "skills.sh".into(),
            id: format!("{owner}/{repo}/{skill}"),
            installed_at: Utc::now(),
        };
        tokio::fs::write(
            staging.join(PROVENANCE_FILE),
            serde_json::to_vec_pretty(&provenance).unwrap_or_default(),
        )
        .await
        .map_err(|error| format!("Couldn't save the install: {error}"))?;
        tokio::fs::create_dir_all(root)
            .await
            .map_err(|error| format!("Couldn't create {}: {error}", root.display()))?;
        // Checked again at the last moment: an install racing this one, or a
        // folder made by hand, keeps its contents.
        if tokio::fs::symlink_metadata(&target).await.is_ok() {
            return Err(format!("A skill named {skill} is already installed."));
        }
        tokio::fs::rename(&staging, &target)
            .await
            .map_err(|error| format!("Couldn't install {}: {error}", manifest.name))?;
        library_skill(&target).ok_or_else(|| "This skill's SKILL.md isn't valid.".to_string())
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let _ = tokio::fs::remove_dir(&staging_parent).await;
    result
}

async fn write_file(path: &Path, bytes: &[u8], executable: bool) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(dir: &Path, name: &str, description: &str) -> PathBuf {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join(SKILL_FILE),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\nBody.\n"),
        )
        .unwrap();
        folder
    }

    #[test]
    fn lists_skills_grouped_one_level_down_and_skips_hidden_folders() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        skill(&root, "beta", "Second");
        skill(&root, "Alpha", "First");
        skill(&root.join("group"), "nested", "Grouped");
        skill(&root.join(".hidden"), "ghost", "Hidden");
        std::fs::write(root.join("notes.md"), "not a skill").unwrap();
        let library = list_at(&root).unwrap();
        let names: Vec<_> = library.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "beta", "nested"]);
        assert_eq!(library.skills[0].description, "First");
        assert!(library.skills[0].file.ends_with("Alpha/SKILL.md"));
        assert_eq!(library.skills[0].registry_id, None);
    }

    #[test]
    fn a_missing_root_is_an_empty_library() {
        let temp = tempfile::tempdir().unwrap();
        let library = list_at(&temp.path().join("absent")).unwrap();
        assert!(library.skills.is_empty());
    }

    #[test]
    fn provenance_marks_registry_installs() {
        let temp = tempfile::tempdir().unwrap();
        let folder = skill(temp.path(), "pdf", "PDFs");
        std::fs::write(
            folder.join(PROVENANCE_FILE),
            r#"{"registry":"skills.sh","id":"anthropics/skills/pdf","installedAt":"2026-10-16T00:00:00Z"}"#,
        )
        .unwrap();
        let library = list_at(temp.path()).unwrap();
        assert_eq!(
            library.skills[0].registry_id.as_deref(),
            Some("anthropics/skills/pdf")
        );
    }

    #[test]
    fn contents_strip_frontmatter_and_list_files_skill_first() {
        let temp = tempfile::tempdir().unwrap();
        let folder = skill(temp.path(), "tool", "Tools");
        std::fs::create_dir_all(folder.join("scripts")).unwrap();
        std::fs::write(folder.join("scripts/run.sh"), "echo").unwrap();
        std::fs::write(folder.join("README.md"), "readme").unwrap();
        std::fs::write(folder.join(PROVENANCE_FILE), "{}").unwrap();
        let contents = contents_at(temp.path(), &folder).unwrap();
        assert_eq!(contents.markdown, "# tool\n\nBody.\n");
        assert_eq!(contents.files, ["SKILL.md", "README.md", "scripts/run.sh"]);
    }

    #[test]
    fn remove_refuses_anything_outside_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let inside = skill(&root, "keep", "Kept");
        let outside = skill(temp.path(), "outside", "Elsewhere");
        assert!(remove_at(&root, &outside).is_err());
        assert!(remove_at(&root, &root).is_err());
        assert!(remove_at(&root, &root.join("keep/../../outside")).is_err());
        assert!(outside.join(SKILL_FILE).exists());
        // Not a skill folder.
        std::fs::create_dir_all(root.join("plain")).unwrap();
        assert!(remove_at(&root, &root.join("plain")).is_err());
        remove_at(&root, &inside).unwrap();
        assert!(!inside.exists());
    }

    #[cfg(unix)]
    #[test]
    fn removing_a_linked_skill_unlinks_it_and_keeps_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(&root).unwrap();
        let target = skill(temp.path(), "mine", "Authored elsewhere");
        let link = root.join("mine");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(list_at(&root).unwrap().skills[0].linked);
        remove_at(&root, &link).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(target.join(SKILL_FILE).exists());
    }

    #[test]
    fn registry_ids_are_github_owner_repo_skill() {
        assert_eq!(
            parse_registry_id("anthropics/skills/pdf").unwrap(),
            ("anthropics", "skills", "pdf")
        );
        assert_eq!(
            parse_registry_id("github.com/vercel-labs/agent.skills/find-skills").unwrap(),
            ("vercel-labs", "agent.skills", "find-skills")
        );
        for bad in [
            "",
            "pdf",
            "anthropics/pdf",
            "mintlify.com/docs/skill",
            "a/b/c/d",
            "a/../c",
            "a/b/..",
            "-a/b/c",
            "a/b/c d",
        ] {
            assert!(parse_registry_id(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn parses_the_home_page_leaderboard() {
        let html = concat!(
            r#"x\"source\":\"vercel-labs/skills\",\"skillId\":\"find-skills\",\"name\":\"find-skills\",\"installs\":3881956}"#,
            r#",{\"source\":\"mintlify.com\",\"skillId\":\"docs\",\"name\":\"docs\",\"installs\":10}"#,
            r#",{\"source\":\"vercel-labs/skills\",\"skillId\":\"find-skills\",\"name\":\"find-skills\",\"installs\":1}"#,
            r#",{\"source\":\"broken"#,
            r#",{\"source\":\"mattpocock/skills\",\"skillId\":\"grill-me\",\"name\":\"grill-me\",\"installs\":1369631}"#,
        );
        let skills = parse_directory(html);
        assert_eq!(
            skills
                .iter()
                .map(|s| (s.id.as_str(), s.installs))
                .collect::<Vec<_>>(),
            [
                ("vercel-labs/skills/find-skills", 3881956),
                ("mattpocock/skills/grill-me", 1369631)
            ]
        );
    }

    fn blob(path: &str, mode: &str, size: u64) -> TreeEntry {
        TreeEntry {
            path: path.into(),
            mode: mode.into(),
            kind: "blob".into(),
            size: Some(size),
        }
    }

    #[test]
    fn finds_the_shallowest_folder_named_after_the_skill() {
        let tree = vec![
            blob("README.md", "100644", 1),
            blob("examples/skills/pdf/SKILL.md", "100644", 1),
            blob("skills/pdf/SKILL.md", "100644", 1),
            blob("skills/docx/SKILL.md", "100644", 1),
        ];
        assert_eq!(skill_folder(&tree, "pdf").as_deref(), Some("skills/pdf"));
        assert_eq!(skill_folder(&tree, "PDF").as_deref(), Some("skills/pdf"));
        assert_eq!(skill_folder(&tree, "xlsx"), None);
        let single = vec![blob("SKILL.md", "100644", 1)];
        assert_eq!(skill_folder(&single, "anything").as_deref(), Some(""));
    }

    #[test]
    fn install_plans_take_regular_files_and_enforce_bounds() {
        let tree = vec![
            blob("skills/pdf/SKILL.md", "100644", 10),
            blob("skills/pdf/scripts/fill.py", "100755", 10),
            blob("skills/pdf/link", "120000", 10),
            blob("skills/pdfx/SKILL.md", "100644", 10),
            TreeEntry {
                path: "skills/pdf/vendor".into(),
                mode: "160000".into(),
                kind: "commit".into(),
                size: None,
            },
        ];
        assert_eq!(
            install_plan(&tree, "skills/pdf").unwrap(),
            [
                ("SKILL.md".to_string(), false),
                ("scripts/fill.py".to_string(), true)
            ]
        );
        let huge = vec![blob("s/SKILL.md", "100644", MAX_INSTALL_FILE_BYTES + 1)];
        assert!(install_plan(&huge, "s").is_err());
        let missing = vec![blob("s/README.md", "100644", 1)];
        assert!(install_plan(&missing, "s").is_err());
        assert!(!safe_relative("../escape"));
        assert!(!safe_relative("a//b"));
        assert!(!safe_relative("a\\b"));
        assert!(safe_relative("scripts/run.sh"));
    }

    /// Live check against skills.sh and GitHub: `cargo test -p zeron-engine
    /// skill_library::tests::live -- --ignored`.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_search_preview_and_install() {
        let popular = search(String::new()).await.unwrap();
        assert!(popular.popular && !popular.skills.is_empty());
        let results = search("pdf".into()).await.unwrap();
        let pdf = results
            .skills
            .iter()
            .find(|skill| skill.id == "anthropics/skills/pdf")
            .expect("anthropics/skills/pdf is searchable");
        let pdf_preview = preview(pdf.id.clone()).await.unwrap();
        assert!(pdf_preview.description.is_some() && !pdf_preview.markdown.is_empty());
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let installed = install_into(&root, &pdf.id).await.unwrap();
        assert_eq!(
            installed.registry_id.as_deref(),
            Some("anthropics/skills/pdf")
        );
        assert!(Path::new(&installed.file).is_file());
        assert!(!temp.path().join(".skills-staging").exists());
        assert!(install_into(&root, &pdf.id).await.is_err());
        // Its folder is `skills/react-best-practices`; only the frontmatter
        // carries this name.
        let renamed = preview("vercel-labs/agent-skills/vercel-react-best-practices".into())
            .await
            .unwrap();
        assert!(!renamed.markdown.is_empty());
        let library = list_at(&root).unwrap();
        assert_eq!(library.skills.len(), 1);
        println!(
            "{:#?}",
            contents_at(&root, Path::new(&installed.directory))
                .unwrap()
                .files
        );
    }

    #[test]
    fn raw_paths_are_percent_encoded_per_segment() {
        assert_eq!(
            encode_path("skills/my skill/SKILL.md"),
            "skills/my%20skill/SKILL.md"
        );
    }
}
