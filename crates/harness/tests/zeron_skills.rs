//! Each harness hands a project's `.zeron/skills` to the agent it launches.
//! The fakes record what they were given in the run's working directory.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use zeron_harness::{
    AcpHarness, CancellationToken, ClaudeHarness, CodexHarness, CursorHarness, Harness, PiHarness,
    RunControls,
};
use zeron_proto::{AgentEvent, DoneStatus, RunRequest, SandboxLevel};

struct Project {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    root: PathBuf,
    skill: PathBuf,
}

fn project() -> Project {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let root = repo.join(".zeron/skills");
    let skill = root.join("ship-it/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        "---\nname: ship-it\ndescription: Ship safely\n---\nBody",
    )
    .unwrap();
    Project {
        _temp: temp,
        repo,
        root,
        skill,
    }
}

fn executable(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Records its argv in the run's cwd, then exits as a failed launch.
fn argv_recorder(dir: &Path, name: &str) -> PathBuf {
    executable(
        dir,
        name,
        "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> argv.txt; done\n\
         case \"$1\" in --version) echo '1.0.0'; exit 0;; esac\nexit 1\n",
    )
}

fn request(cwd: &Path, prompt: &str) -> RunRequest {
    RunRequest {
        mcp: None,
        prompt: prompt.into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: serde_json::Map::new(),
        cwd: cwd.to_str().unwrap().into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: true,
        access: Default::default(),
        plan: false,
        attachments: Vec::new(),
        worktree: None,
        resume: None,
        env: Default::default(),
    }
}

fn controls() -> RunControls {
    let (_steer_tx, steer_rx) = mpsc::channel(8);
    RunControls {
        realtime: None,
        execution_lease: None,
        request_input: Box::new(|_| oneshot::channel().1),
        steering: steer_rx,
        interrupt: CancellationToken::new(),
        turn: Default::default(),
    }
}

async fn run(harness: &impl Harness, req: RunRequest) -> Vec<AgentEvent> {
    let Ok(stream) = harness.run(req, controls()).await else {
        return Vec::new();
    };
    tokio::time::timeout(
        Duration::from_secs(20),
        stream
            .filter_map(|event| async { event.ok() })
            .collect::<Vec<_>>(),
    )
    .await
    .expect("run finished in time")
}

/// One test, so the process env is set before any other thread exists: no
/// case reads the real home's `~/.zeron/skills` or writes under it.
#[test]
fn every_harness_loads_project_zeron_skills_at_launch() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary, before its runtime starts threads.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("ZERON_DATA_DIR", home.path().join("data"));
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(pi_launches_with_each_root_as_a_skill_nearest_first());
    runtime.block_on(claude_launches_with_a_zeron_plugin_linking_the_skill(
        home.path(),
    ));
    runtime.block_on(codex_sets_the_roots_after_initialized());
    runtime.block_on(fx_lists_the_skills_in_each_fresh_sessions_first_prompt_only());
    runtime.block_on(cursor_lists_the_skills_in_the_first_run_prompt());
}

async fn pi_launches_with_each_root_as_a_skill_nearest_first() {
    let project = project();
    let nested = project.repo.join("packages/web");
    std::fs::create_dir_all(nested.join(".zeron/skills")).unwrap();
    let exe = argv_recorder(project.repo.parent().unwrap(), "pi");
    run(
        &PiHarness::new().with_executable(exe),
        request(&nested, "hi"),
    )
    .await;
    let argv = std::fs::read_to_string(nested.join("argv.txt")).unwrap();
    let roots: Vec<&Path> = argv
        .lines()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|pair| pair[0] == "--skill")
        .map(|pair| Path::new(pair[1]))
        .collect();
    let nearest = roots
        .iter()
        .position(|r| *r == nested.join(".zeron/skills"))
        .expect(&argv);
    let project_root = roots.iter().position(|r| *r == project.root).expect(&argv);
    assert!(nearest < project_root, "{argv}");
}

async fn claude_launches_with_a_zeron_plugin_linking_the_skill(home: &Path) {
    let project = project();
    let exe = argv_recorder(project.repo.parent().unwrap(), "claude");
    run(
        &ClaudeHarness::new().with_executable(exe),
        request(&project.repo, "hi"),
    )
    .await;
    let argv = std::fs::read_to_string(project.repo.join("argv.txt")).unwrap();
    let lines: Vec<&str> = argv.lines().collect();
    let at = lines
        .iter()
        .position(|line| *line == "--plugin-dir")
        .expect(&argv);
    let plugin = Path::new(lines[at + 1]);
    assert!(
        plugin.starts_with(home.join("data/claude-skills")),
        "{argv}"
    );
    let manifest = std::fs::read_to_string(plugin.join(".claude-plugin/plugin.json")).unwrap();
    assert!(manifest.contains(r#""name":"zeron""#), "{manifest}");
    assert_eq!(
        std::fs::canonicalize(plugin.join("skills/ship-it/SKILL.md")).unwrap(),
        std::fs::canonicalize(&project.skill).unwrap()
    );
}

async fn codex_sets_the_roots_after_initialized() {
    let project = project();
    let exe = executable(
        project.repo.parent().unwrap(),
        "codex",
        r#"#!/usr/bin/env python3
import json, sys
if '--version' in sys.argv:
    print('codex-cli 0.200.0'); sys.exit(0)
log = open('codex.log', 'a')
for line in sys.stdin:
    msg = json.loads(line)
    log.write(line); log.flush()
    if msg.get('method') == 'skills/extraRoots/set':
        sys.exit(1)
    if 'id' in msg:
        print(json.dumps({'id': msg['id'], 'result': {}}), flush=True)
"#,
    );
    run(
        &CodexHarness::new().with_executable(exe),
        request(&project.repo, "hi"),
    )
    .await;
    let log = std::fs::read_to_string(project.repo.join("codex.log")).unwrap();
    let methods: Vec<serde_json::Value> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let initialized = methods
        .iter()
        .position(|m| m["method"] == "initialized")
        .expect(&log);
    let set = methods
        .iter()
        .position(|m| m["method"] == "skills/extraRoots/set")
        .expect(&log);
    assert!(initialized < set, "{log}");
    let roots = methods[set]["params"]["extraRoots"].as_array().unwrap();
    assert!(
        roots
            .iter()
            .any(|root| Path::new(root.as_str().unwrap()) == project.root),
        "{log}"
    );
}

async fn fx_lists_the_skills_in_each_fresh_sessions_first_prompt_only() {
    let project = project();
    let exe = executable(
        project.repo.parent().unwrap(),
        "fx",
        r#"#!/usr/bin/env python3
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    method, params, result = msg.get('method'), msg.get('params', {}), {}
    if method == 'initialize':
        result = dict(protocolVersion=1, agentCapabilities=dict(loadSession=True))
    elif method == 'session/load' and params.get('sessionId') == 'gone':
        print(json.dumps(dict(id=msg['id'], error=dict(code=-32002, message='session not found'))), flush=True)
        continue
    elif method in ('session/new', 'session/load'):
        result = dict(sessionId='fx-session')
    elif method == 'session/prompt':
        with open('prompts.jsonl', 'a') as f:
            f.write(json.dumps(params['prompt'][0]['text']) + '\n')
        result = dict(stopReason='end_turn')
    if 'id' in msg and method is not None:
        print(json.dumps(dict(id=msg['id'], result=result)), flush=True)
"#,
    );
    let harness = AcpHarness::fx().with_executable(exe);
    for resume in [
        None,
        Some("fx-session".to_string()),
        Some("gone".to_string()),
    ] {
        let mut req = request(&project.repo, "hello");
        req.resume = resume;
        let events = run(&harness, req).await;
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Done {
                    status: DoneStatus::Completed,
                    ..
                }
            )),
            "{events:?}"
        );
    }
    let prompts: Vec<String> = std::fs::read_to_string(project.repo.join("prompts.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(prompts.len(), 3, "{prompts:?}");
    assert!(prompts[0].starts_with("<zeron-skills>"), "{}", prompts[0]);
    let listed = format!(
        "- ship-it: Ship safely (SKILL.md: {})",
        project.skill.display()
    );
    assert!(prompts[0].contains(&listed), "{}", prompts[0]);
    assert!(
        prompts[0].ends_with("</zeron-skills>\n\nhello"),
        "{}",
        prompts[0]
    );
    assert_eq!(prompts[1], "hello");
    assert_eq!(
        prompts[2], prompts[0],
        "a resume that starts fresh lists the skills"
    );
}

async fn cursor_lists_the_skills_in_the_first_run_prompt() {
    let project = project();
    let exe = executable(
        project.repo.parent().unwrap(),
        "cursor-shim",
        r#"#!/usr/bin/env python3
import json, os, sys
first = json.loads(sys.stdin.readline())
with open(os.path.join(first['cwd'], 'first.json'), 'w') as f:
    json.dump(first, f)
print(json.dumps(dict(ev='fatal', message='recorded')), flush=True)
"#,
    );
    run(
        &CursorHarness::new().with_executable(exe),
        request(&project.repo, "hello"),
    )
    .await;
    let first: serde_json::Value =
        serde_json::from_slice(&std::fs::read(project.repo.join("first.json")).unwrap()).unwrap();
    let prompt = first["prompt"].as_str().unwrap();
    let listed = format!(
        "- ship-it: Ship safely (SKILL.md: {})",
        project.skill.display()
    );
    assert!(
        prompt.starts_with("<zeron-skills>") && prompt.contains(&listed),
        "{prompt}"
    );
    assert!(prompt.ends_with("\n\nhello"), "{prompt}");
}
