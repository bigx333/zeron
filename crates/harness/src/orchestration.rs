//! Native app guidance and the CLI routing capability share the injected MCP origin.
use std::{collections::BTreeMap, path::Path};
use zeron_proto::McpServer;

pub(crate) fn instructions(server: Option<&McpServer>) -> Option<&'static str> {
    server
        .filter(|s| s.name == "zeron")
        .map(|_| zeron_proto::ORCHESTRATION_INSTRUCTIONS)
}

pub(crate) fn routing_env(server: Option<&McpServer>) -> BTreeMap<String, String> {
    let Some(server) = server.filter(|s| s.name == "zeron") else {
        return BTreeMap::new();
    };
    let mut env: BTreeMap<_, _> = server
        .env
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "ZERON_CHAT_ID" | "ZERON_DEVICE_ID" | "ZERON_IPC_PORT" | "ZERON_CLI"
            )
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    env.entry("ZERON_CLI".into())
        .or_insert_with(|| server.command.clone());
    env
}

pub(crate) fn apply_env(command: &mut crate::process::Command, server: Option<&McpServer>) {
    // A nested engine must not inherit another chat's routing capability.
    for key in [
        "ZERON_CHAT_ID",
        "ZERON_DEVICE_ID",
        "ZERON_IPC_PORT",
        "ZERON_CLI",
    ] {
        command.env_remove(key);
    }
    let env = routing_env(server);
    let cli = env.get("ZERON_CLI").and_then(|path| {
        Path::new(path)
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
    });
    if let Some(dir) = cli {
        let current = command
            .as_std_mut()
            .get_envs()
            .find_map(|(k, v)| (k == "PATH").then(|| v.map(|v| v.to_owned())).flatten())
            .or_else(|| std::env::var_os("PATH"));
        let paths = std::iter::once(dir.to_path_buf()).chain(
            current
                .as_deref()
                .into_iter()
                .flat_map(std::env::split_paths),
        );
        if let Ok(path) = std::env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
    for (key, value) in env {
        command.env(key, value);
    }
}

/// Codex's sandboxed shell has its own environment, separate from app-server.
pub(crate) fn shell_env(server: Option<&McpServer>) -> BTreeMap<String, String> {
    let mut env = routing_env(server);
    if let Some(cli) = env.get("ZERON_CLI")
        && let Some(dir) = Path::new(cli)
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
    {
        let mut paths = vec![dir.to_path_buf()];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        if let Some(path) = crate::shell_env::login_shell_path() {
            paths.extend(std::env::split_paths(path));
        }
        if let Ok(path) = std::env::join_paths(paths) {
            env.insert("PATH".into(), path.to_string_lossy().into_owned());
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_is_scoped_and_does_not_expose_unrelated_mcp_environment() {
        let server = McpServer {
            name: "zeron".into(),
            command: "/opt/zeron/zeron".into(),
            args: vec!["mcp".into()],
            env: [
                ("ZERON_CHAT_ID".into(), "parent".into()),
                ("UNRELATED_SECRET".into(), "private".into()),
            ]
            .into_iter()
            .collect(),
        };
        let env = shell_env(Some(&server));
        assert_eq!(env.get("ZERON_CHAT_ID").unwrap(), "parent");
        assert_eq!(env.get("ZERON_CLI").unwrap(), "/opt/zeron/zeron");
        assert!(!env.contains_key("UNRELATED_SECRET"));
        assert_eq!(
            std::env::split_paths(env.get("PATH").unwrap())
                .next()
                .unwrap(),
            Path::new("/opt/zeron")
        );
        assert!(
            instructions(Some(&server))
                .unwrap()
                .contains("eight active child tasks")
        );
        let external = McpServer {
            name: "external".into(),
            ..server
        };
        assert!(routing_env(Some(&external)).is_empty());
        assert!(instructions(Some(&external)).is_none());
    }
}
