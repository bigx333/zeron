//! Zeron's own skill folders, `~/.zeron/skills` and `<project>/.zeron/skills`.
//! Each harness gets the existing roots at launch in its own dialect, so the
//! agent loads them natively rather than only when one is selected.
use super::{MAX_DISCOVERY_ENTRIES, project_dirs, scan_root};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};
use zeron_proto::{
    HarnessId, SlashCommand,
    invocation::{Skill, SkillCommand},
};

pub(crate) const DIR: &str = ".zeron/skills";

/// The Claude plugin that carries Zeron's skills; they surface as `/zeron:<name>`.
pub(crate) const CLAUDE_PLUGIN: &str = "zeron";

/// Existing roots for `cwd`: global first, then project roots from the
/// worktree root down, so a later root overrides an earlier one by name.
pub(crate) fn roots(cwd: &Path) -> Vec<PathBuf> {
    roots_at(cwd, &crate::executable::home_or_current_dir())
}

pub(crate) fn roots_at(cwd: &Path, home: &Path) -> Vec<PathBuf> {
    let mut scope = Vec::new();
    for dir in cwd.ancestors() {
        scope.push(dir);
        if dir.join(".git").exists() || dir == home {
            break;
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for dir in std::iter::once(home).chain(scope.into_iter().rev()) {
        let root = dir.join(DIR);
        if root.is_dir() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// Folder skills under `roots`, keyed by name; the nearest root wins.
pub(crate) fn skills(roots: &[PathBuf]) -> Vec<Skill> {
    let mut found = BTreeMap::new();
    let mut remaining = MAX_DISCOVERY_ENTRIES;
    for root in roots {
        let _ = scan_root(root, "", HarnessId::ClaudeCode, &mut found, &mut remaining);
    }
    found.into_values().collect()
}

/// The provider's native project folders, then Zeron's, so Zeron's wins a
/// name collision at the same scope.
pub(super) fn dirs(harness: HarnessId) -> impl Iterator<Item = &'static str> {
    let own = (harness != HarnessId::Mock).then_some(DIR);
    project_dirs(harness).iter().copied().chain(own)
}

pub(super) fn is_root(root: &Path) -> bool {
    root.ends_with(DIR)
}

pub(crate) fn is_zeron_skill(path: &str) -> bool {
    Path::new(path).ancestors().any(is_root)
}

/// Pi: one `--skill <root>` per root, beside Pi's native folders. Pi keeps
/// the first of two same-named skills, so the nearest root goes first.
pub(crate) fn pi_args(cwd: &Path) -> Vec<OsString> {
    roots(cwd)
        .into_iter()
        .rev()
        .flat_map(|root| [OsString::from("--skill"), root.into_os_string()])
        .collect()
}

/// Codex: `skills/extraRoots/set` params for the app-server, if any root exists.
pub(crate) fn codex_extra_roots(cwd: &Path) -> Option<Value> {
    let roots = roots(cwd);
    (!roots.is_empty()).then(|| json!({ "extraRoots": roots }))
}

/// Codex: add the roots to this app-server process. Older Codex builds lack
/// the method; their skills stay selectable as file references.
pub(crate) async fn codex_set_roots(client: &crate::jsonrpc::RpcClient, cwd: &Path) {
    let Some(params) = codex_extra_roots(cwd) else {
        return;
    };
    let request = client.request("skills/extraRoots/set", params);
    match tokio::time::timeout(std::time::Duration::from_secs(10), request).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(%error, "Codex did not accept Zeron skill roots"),
        Err(_) => tracing::warn!("Codex did not answer skills/extraRoots/set"),
    }
}

/// OpenCode: append the roots to an inline config's `skills` paths, a plain
/// list on 2.x and the `{paths}` object on 1.x.
pub(crate) fn opencode_config(
    config: &mut Value,
    roots: &[PathBuf],
    v1: bool,
) -> Result<(), &'static str> {
    let object = config.as_object_mut().ok_or("config must be an object")?;
    let skills = object
        .entry("skills")
        .or_insert_with(|| if v1 { json!({"paths": []}) } else { json!([]) });
    let paths = match skills {
        Value::Array(paths) => paths,
        Value::Object(object) => object
            .entry("paths")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("skills.paths must be a list")?,
        _ => return Err("skills must be a list or object"),
    };
    paths.extend(roots.iter().map(|root| json!(root)));
    Ok(())
}

/// Claude Code loads extra skills only from plugins, so `--plugin-dir` names a
/// session-only plugin whose `skills/` links each Zeron skill.
pub(crate) fn claude_args(cwd: &Path) -> Vec<OsString> {
    let base = std::env::temp_dir().join("zeron-claude-skills");
    match claude_plugin(&roots(cwd), &base) {
        Ok(Some(plugin)) => vec!["--plugin-dir".into(), plugin.into_os_string()],
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(%error, "could not assemble the Zeron skills plugin");
            Vec::new()
        }
    }
}

/// The plugin directory is named by a hash of its links, so concurrent runs
/// with the same skills share one and a changed set gets a fresh one.
pub(crate) fn claude_plugin(roots: &[PathBuf], base: &Path) -> std::io::Result<Option<PathBuf>> {
    use sha2::Digest;
    let links: Vec<(String, PathBuf)> = skills(roots)
        .into_iter()
        .filter(|skill| {
            !skill.name.starts_with('.')
                && skill
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .filter_map(|skill| Some((skill.name, Path::new(&skill.path).parent()?.to_owned())))
        .collect();
    if links.is_empty() {
        return Ok(None);
    }
    let mut hash = sha2::Sha256::new();
    for (name, dir) in &links {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(dir.as_os_str().as_encoded_bytes());
        hash.update([0]);
    }
    let digest: String = hash.finalize()[..12]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let plugin = base.join(digest);
    let manifest = Path::new(".claude-plugin/plugin.json");
    if plugin.join(manifest).is_file() {
        return Ok(Some(plugin));
    }
    std::fs::create_dir_all(base)?;
    let staging = base.join(format!(".{}", uuid::Uuid::new_v4()));
    let result = (|| {
        std::fs::create_dir_all(staging.join(".claude-plugin"))?;
        std::fs::write(
            staging.join(manifest),
            json!({
                "name": CLAUDE_PLUGIN,
                "version": "0.1.0",
                "description": "Skills from Zeron's .zeron/skills folders.",
            })
            .to_string(),
        )?;
        std::fs::create_dir(staging.join("skills"))?;
        for (name, dir) in &links {
            #[cfg(unix)]
            std::os::unix::fs::symlink(dir, staging.join("skills").join(name))?;
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(dir, staging.join("skills").join(name))?;
        }
        std::fs::rename(&staging, &plugin)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
        // Another run may have published the same content first.
        if !plugin.join(manifest).is_file() {
            result?;
        }
    }
    Ok(Some(plugin))
}

/// Claude: a Zeron skill the CLI advertised as `zeron:<name>` invokes natively.
pub(crate) fn bind_claude_commands(
    mut skills: Vec<Skill>,
    commands: &[SlashCommand],
    harness: HarnessId,
) -> Vec<Skill> {
    for skill in &mut skills {
        let name = format!("{CLAUDE_PLUGIN}:{}", skill.name);
        if is_zeron_skill(&skill.path) && commands.iter().any(|command| command.name == name) {
            skill.command = Some(SkillCommand { name, harness });
        }
    }
    skills
}

/// Agents without a configurable skill root (ACP agents, Cursor) learn the
/// skills from a fresh session's first prompt. A leading slash command stays
/// first so the agent still routes it.
pub(crate) fn with_instructions(cwd: &str, resumed: bool, prompt: String) -> String {
    if resumed || cwd.is_empty() {
        return prompt;
    }
    with_instructions_for(&roots(Path::new(cwd)), prompt)
}

fn with_instructions_for(roots: &[PathBuf], prompt: String) -> String {
    if zeron_proto::invocation::leading_command(&prompt).is_some() {
        return prompt;
    }
    match instructions(roots) {
        Some(block) => format!("{block}\n\n{prompt}"),
        None => prompt,
    }
}

/// Name, description and absolute `SKILL.md` path, read on demand by the agent.
fn instructions(roots: &[PathBuf]) -> Option<String> {
    let lines: Vec<String> = skills(roots)
        .into_iter()
        .filter(|skill| skill.enabled)
        .map(|skill| {
            let description = skill
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let description = if description.is_empty() {
                "(no description)".into()
            } else {
                description
            };
            format!("- {}: {description} (SKILL.md: {})", skill.name, skill.path)
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(format!(
        "<zeron-skills>\nZeron skills are reusable instruction folders. When the current task \
         matches a listed skill description, read that skill's SKILL.md at the absolute path \
         before proceeding; you may read supporting files in the same skill directory that \
         SKILL.md references.\n\n{}\n</zeron-skills>",
        lines.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{discover_at, is_shared_skill};
    use super::*;

    fn write(root: &Path, relative: &str, body: &str) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn roots_are_global_first_nearest_last_and_stop_at_the_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        let nested = repo.join("packages/web");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        for dir in [&home, &repo, &nested, &temp.path().to_path_buf()] {
            std::fs::create_dir_all(dir.join(DIR)).unwrap();
        }
        assert_eq!(
            roots_at(&nested, &home),
            [home.join(DIR), repo.join(DIR), nested.join(DIR)]
        );
        std::fs::remove_dir(repo.join(DIR)).unwrap();
        assert_eq!(roots_at(&repo, &home), [home.join(DIR)]);
        assert_eq!(roots_at(&home, &home), [home.join(DIR)]);
    }

    #[test]
    fn the_nearest_root_wins_a_name() {
        let temp = tempfile::tempdir().unwrap();
        let global = temp.path().join("home").join(DIR);
        let project = temp.path().join("repo").join(DIR);
        write(
            &global,
            "review/SKILL.md",
            "---\nname: review\ndescription: Global\n---\n",
        );
        write(&global, "deploy/SKILL.md", "---\nname: deploy\n---\n");
        let nearest = write(
            &project,
            "review/SKILL.md",
            "---\nname: review\ndescription: Project\n---\n",
        );
        let found = skills(&[global, project]);
        assert_eq!(
            found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["deploy", "review"]
        );
        assert_eq!(Path::new(&found[1].path), nearest);
        assert_eq!(found[1].description, "Project");
    }

    #[test]
    fn every_harness_lists_global_and_project_zeron_skills() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        write(
            &home,
            ".zeron/skills/global/SKILL.md",
            "---\nname: global\n---\nBody",
        );
        write(&home, ".zeron/skills/loose.md", "Not a skill folder");
        write(
            &repo,
            ".agents/skills/review/SKILL.md",
            "---\nname: review\ndescription: Shared\n---\n",
        );
        let project = write(
            &repo,
            ".zeron/skills/review/SKILL.md",
            "---\nname: review\ndescription: Zeron\n---\n",
        );
        for harness in [
            HarnessId::ClaudeCode,
            HarnessId::Codex,
            HarnessId::Cursor,
            HarnessId::Devin,
            HarnessId::Grok,
            HarnessId::Hermes,
            HarnessId::Fx,
            HarnessId::Pi,
            HarnessId::Antigravity,
            HarnessId::Opencode,
            HarnessId::Muse,
        ] {
            let skills = discover_at(harness, &repo, &home).unwrap();
            assert!(skills.iter().any(|s| s.name == "global"), "{harness:?}");
            assert!(!skills.iter().any(|s| s.name == "loose"), "{harness:?}");
            let review = skills.iter().find(|s| s.name == "review").unwrap();
            assert_eq!(review.description, "Zeron", "{harness:?}");
            assert_eq!(Path::new(&review.path), project.as_path());
        }
        assert!(
            discover_at(HarnessId::Mock, &repo, &home)
                .unwrap()
                .is_empty()
        );
        assert!(is_zeron_skill(&project.to_string_lossy()));
        assert!(is_shared_skill(&project.to_string_lossy()));
        assert!(!is_zeron_skill("/repo/.agents/skills/review/SKILL.md"));
    }

    #[cfg(unix)]
    #[test]
    fn claude_plugin_links_the_nearest_skill_and_is_shared_by_content() {
        let temp = tempfile::tempdir().unwrap();
        let global = temp.path().join("home").join(DIR);
        let project = temp.path().join("repo").join(DIR);
        write(&global, "review/SKILL.md", "---\nname: review\n---\nGlobal");
        write(&global, "deploy/SKILL.md", "---\nname: deploy\n---\nBody");
        write(&global, "bad/SKILL.md", "---\nname: a:b\n---\nBody");
        write(
            &project,
            "review/SKILL.md",
            "---\nname: review\n---\nProject",
        );
        let base = temp.path().join("plugins");
        let roots = [global, project.clone()];
        let plugin = claude_plugin(&roots, &base).unwrap().unwrap();
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(plugin.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"], CLAUDE_PLUGIN);
        let mut linked: Vec<_> = std::fs::read_dir(plugin.join("skills"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        linked.sort();
        assert_eq!(linked, ["deploy", "review"]);
        assert_eq!(
            std::fs::read_to_string(plugin.join("skills/review/SKILL.md")).unwrap(),
            "---\nname: review\n---\nProject"
        );
        assert_eq!(claude_plugin(&roots, &base).unwrap().unwrap(), plugin);
        write(&project, "lint/SKILL.md", "---\nname: lint\n---\nBody");
        assert_ne!(claude_plugin(&roots, &base).unwrap().unwrap(), plugin);
        assert!(claude_plugin(&[], &base).unwrap().is_none());
    }

    #[test]
    fn claude_binds_only_advertised_plugin_commands_of_zeron_skills() {
        let skill = |name: &str, path: &str| Skill {
            name: name.into(),
            path: path.into(),
            description: String::new(),
            enabled: true,
            command: None,
        };
        let command = |name: &str| SlashCommand {
            name: name.into(),
            description: String::new(),
            input_hint: None,
        };
        let bound = bind_claude_commands(
            vec![
                skill("deploy", "/r/.zeron/skills/deploy/SKILL.md"),
                skill("lint", "/r/.zeron/skills/lint/SKILL.md"),
                skill("review", "/r/.agents/skills/review/SKILL.md"),
            ],
            &[command("zeron:deploy"), command("zeron:review")],
            HarnessId::ClaudeCode,
        );
        let names: Vec<_> = bound
            .iter()
            .map(|s| s.command.as_ref().map(|c| c.name.as_str()))
            .collect();
        assert_eq!(names, [Some("zeron:deploy"), None, None]);
    }

    #[test]
    fn pi_codex_and_opencode_get_every_root_in_their_own_shape() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join(DIR)).unwrap();
        let root = repo.join(DIR);
        let args = pi_args(&repo);
        let at = args
            .iter()
            .position(|arg| *arg == *root.as_os_str())
            .unwrap();
        assert_eq!(args[at - 1], "--skill");
        let params = codex_extra_roots(&repo).unwrap();
        assert!(
            params["extraRoots"]
                .as_array()
                .unwrap()
                .contains(&json!(root))
        );

        let roots = [PathBuf::from("/home/u/.zeron/skills")];
        let mut v2 = json!({"model": "x"});
        opencode_config(&mut v2, &roots, false).unwrap();
        assert_eq!(
            v2,
            json!({"model": "x", "skills": ["/home/u/.zeron/skills"]})
        );
        let mut v1 = json!({});
        opencode_config(&mut v1, &roots, true).unwrap();
        assert_eq!(v1["skills"]["paths"], json!(["/home/u/.zeron/skills"]));
        let mut inherited = json!({"skills": ["/team"]});
        opencode_config(&mut inherited, &roots, true).unwrap();
        assert_eq!(
            inherited["skills"],
            json!(["/team", "/home/u/.zeron/skills"])
        );
        let mut legacy = json!({"skills": {"paths": ["/team"]}});
        opencode_config(&mut legacy, &roots, false).unwrap();
        assert_eq!(
            legacy["skills"]["paths"],
            json!(["/team", "/home/u/.zeron/skills"])
        );
        assert!(opencode_config(&mut json!({"skills": 1}), &roots, false).is_err());
    }

    #[test]
    fn instructions_list_skills_only_for_fresh_non_command_prompts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(DIR);
        let path = write(
            &root,
            "deploy/SKILL.md",
            "---\nname: deploy\ndescription: >-\n  Ship it\n  safely\n---\nBody",
        );
        write(
            &root,
            "hidden/SKILL.md",
            "---\nname: hidden\nuser-invocable: false\n---\nBody",
        );
        let roots = [root];
        let prompt = with_instructions_for(&roots, "hello".into());
        assert!(prompt.starts_with("<zeron-skills>"), "{prompt}");
        let skill_path = prompt
            .lines()
            .find_map(|line| {
                line.strip_prefix("- deploy: Ship it safely (SKILL.md: ")?
                    .strip_suffix(')')
            })
            .unwrap_or_else(|| panic!("missing deploy skill: {prompt}"));
        // Discovery and fixture paths can mix `/` and `\` differently on Windows.
        assert_eq!(Path::new(skill_path), path, "{prompt}");
        assert!(!prompt.contains("hidden"));
        assert!(prompt.ends_with("</zeron-skills>\n\nhello"));
        assert_eq!(with_instructions_for(&roots, "/compact".into()), "/compact");
        assert_eq!(with_instructions_for(&[], "hi".into()), "hi");
        assert_eq!(
            with_instructions(temp.path().to_str().unwrap(), true, "hi".into()),
            "hi"
        );
        assert_eq!(with_instructions("", false, "hi".into()), "hi");
    }
}
