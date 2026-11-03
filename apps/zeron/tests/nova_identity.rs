//! A Nova build's defaults are its own: with no `ZERON_*` overrides the
//! engine keeps its data, lock and port apart from upstream Zeron's, and the
//! links it hands out open Nova.

use std::process::Command;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
fn engine_defaults_to_novas_data_dir_lock_and_port() {
    let home = tempfile::tempdir().unwrap();
    let upstream = home.path().join(".zeron");
    let legacy = home.path().join(".comet-native");
    std::fs::create_dir_all(&upstream).unwrap();
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(upstream.join("marker"), "upstream").unwrap();
    std::fs::write(legacy.join("marker"), "legacy").unwrap();
    let data_dir = home.path().join(".zeron-nova");

    let mut child = Command::new(env!("CARGO_BIN_EXE_zeron-engine"))
        .env("HOME", home.path())
        .env_remove("ZERON_DATA_DIR")
        .env_remove("ZERON_IPC_PORT")
        .env_remove("ZERON_WORKTREES_DIR")
        .env("ZERON_WORKOS_CLIENT_ID", "")
        .env_remove("ZERON_EDGE_TOKEN")
        .env("ZERON_EDGE_URL", "http://127.0.0.1:1")
        .env("ZERON_DISABLE_NOTIFICATIONS", "1")
        .env("ZERON_AUTO_UPDATE", "0")
        .spawn()
        .expect("start the background engine");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let served = runtime.block_on(async {
        let deadline = Instant::now() + Duration::from_secs(60);
        let client = loop {
            match zeron_rpc::connect_ipc(27664, Some(&data_dir)).await {
                Ok(client) => break client,
                Err(err) if Instant::now() >= deadline => {
                    return Err(format!("never served Nova's port: {err}"));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        };
        client
            .call(zeron_rpc::methods::STOP_ENGINE, serde_json::json!({}))
            .await
            .map_err(|err| format!("StopEngine: {err}"))?;
        Ok(())
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline || served.is_err() {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    served.unwrap();

    assert!(
        data_dir.join("engine.lock").is_file(),
        "lock in Nova's data dir"
    );
    assert!(
        data_dir.join("ipc-token").is_file(),
        "IPC token in Nova's data dir"
    );
    let upstream_entries: Vec<_> = std::fs::read_dir(&upstream)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        upstream_entries,
        ["marker"],
        "upstream's ~/.zeron untouched"
    );
    assert_eq!(
        std::fs::read_to_string(legacy.join("marker")).unwrap(),
        "legacy",
        "upstream's pre-rename data not adopted"
    );
}

#[test]
fn links_open_nova() {
    assert_eq!(
        zeron_ui::links::zeron_conversation_link("c", "w"),
        "zeron-nova://open/chat/c?workspace=w"
    );
    assert_eq!(zeron_notify::link("c"), "zeron-nova://notification/c");
}
