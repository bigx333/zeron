//! Durable, private Git object storage. Neither capture nor restore writes the
//! checkout's index, HEAD, refs, config, or object database. Ignored files are
//! excluded by the checkout's own Git; an ignored obstruction refuses restore.
use crate::EngineError;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::io::AsyncWriteExt;

fn error(s: impl Into<String>) -> EngineError {
    EngineError::Other(s.into())
}

pub(super) async fn git(
    root: &Path,
    store: Option<&Path>,
    index: Option<&Path>,
    args: &[&str],
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, EngineError> {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(root)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Never inherit a caller's temporary-index/object-store environment.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(key);
    }
    if let Some(store) = store {
        command.env("GIT_DIR", store).env("GIT_WORK_TREE", root);
        command.args(["-c", "core.fsync=all", "-c", "core.fsyncMethod=fsync"]);
    }
    command.args(args);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command.spawn()?;
    let writer = input.map(|bytes| {
        let mut stdin = child.stdin.take().unwrap();
        tokio::spawn(async move { stdin.write_all(&bytes).await })
    });
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| error("Checkpoint Git operation timed out"))??;
    if let Some(writer) = writer {
        writer.await.map_err(|e| error(e.to_string()))??;
    }
    if !output.status.success() {
        return Err(error(format!(
            "Checkpoint Git operation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

#[cfg(unix)]
fn path(bytes: &[u8]) -> Result<PathBuf, EngineError> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(bytes.to_vec()).into())
}
#[cfg(not(unix))]
fn path(bytes: &[u8]) -> Result<PathBuf, EngineError> {
    Ok(PathBuf::from(
        std::str::from_utf8(bytes).map_err(|_| error("Checkpoint path is not UTF-8"))?,
    ))
}

fn safe(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, std::path::Component::Normal(s) if !s.to_string_lossy().eq_ignore_ascii_case(".git")))
}

pub(super) async fn capture(root: &Path, store: &Path, id: &str) -> Result<String, EngineError> {
    tokio::fs::create_dir_all(store).await?;
    if !store.join("HEAD").exists() {
        git(
            root,
            None,
            None,
            &[
                "init",
                "--bare",
                "--quiet",
                store
                    .to_str()
                    .ok_or_else(|| error("Invalid checkpoint store path"))?,
            ],
            None,
        )
        .await?;
    }
    tokio::fs::create_dir_all(store.join("info")).await?;
    tokio::fs::write(
        store.join("info/attributes"),
        b"* -text -filter -ident -working-tree-encoding\n",
    )
    .await?;
    let listed = git(
        root,
        None,
        None,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        None,
    )
    .await?;
    let staged = git(root, None, None, &["ls-files", "--stage", "-z"], None).await?;
    if staged
        .split(|b| *b == 0)
        .any(|entry| entry.starts_with(b"160000 "))
    {
        return Err(error("Checkpoint cannot snapshot submodules"));
    }
    let mut selected = Vec::new();
    let mut bytes = 0u64;
    let mut count = 0usize;
    for raw_name in listed.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let name = path(raw_name)?;
        if !safe(&name) {
            return Err(error("Unsafe checkpoint path"));
        }
        for ancestor in name
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            if tokio::fs::symlink_metadata(root.join(ancestor))
                .await
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                return Err(error("A symlink ancestor prevents a complete checkpoint"));
            }
        }
        match tokio::fs::symlink_metadata(root.join(&name)).await {
            Ok(meta) => {
                if meta.is_dir() {
                    continue;
                }
                if !meta.is_file() && !meta.file_type().is_symlink() {
                    return Err(error(
                        "Checkpoint cannot snapshot submodules or special files",
                    ));
                }
                bytes = bytes.saturating_add(meta.len());
                count += 1;
                if bytes > 256 * 1024 * 1024 || count > 50_000 {
                    return Err(error(
                        "Checkout exceeds the complete checkpoint budget (256 MiB / 50000 files)",
                    ));
                }
                selected.extend_from_slice(raw_name);
                selected.push(0);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }
    let index = store.join(format!("capture-{id}.index"));
    let result = async {
        git(
            root,
            Some(store),
            Some(&index),
            &["read-tree", "--empty"],
            None,
        )
        .await?;
        git(
            root,
            Some(store),
            Some(&index),
            &["update-index", "--add", "--remove", "-z", "--stdin"],
            Some(selected),
        )
        .await?;
        let tree =
            String::from_utf8(git(root, Some(store), Some(&index), &["write-tree"], None).await?)
                .map_err(|e| error(e.to_string()))?
                .trim()
                .to_owned();
        git(
            root,
            Some(store),
            None,
            &["update-ref", &format!("refs/zeron/{id}"), &tree],
            None,
        )
        .await?;
        Ok(tree)
    }
    .await;
    let _ = tokio::fs::remove_file(index).await;
    result
}

type Manifest = BTreeMap<PathBuf, String>;
async fn manifest(root: &Path, store: &Path, tree: &str) -> Result<Manifest, EngineError> {
    let listed = git(
        root,
        Some(store),
        None,
        &["ls-tree", "-r", "-z", tree],
        None,
    )
    .await?;
    let mut out = Manifest::new();
    for item in listed.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let at = item
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| error("Malformed checkpoint manifest"))?;
        let name = path(&item[at + 1..])?;
        if !safe(&name) {
            return Err(error("Unsafe checkpoint manifest path"));
        }
        out.insert(name, String::from_utf8_lossy(&item[..at]).into_owned());
    }
    Ok(out)
}

async fn preflight(root: &Path, known: &Manifest, target: &Manifest) -> Result<(), EngineError> {
    // Preflight every obstruction before writing anything. Never follow a
    // checkout symlink while testing an ancestor or replacing a file.
    for name in target.keys() {
        let mut prefix = PathBuf::new();
        for component in name.components() {
            prefix.push(component);
            let Ok(meta) = tokio::fs::symlink_metadata(root.join(&prefix)).await else {
                continue;
            };
            if !meta.is_dir() && !known.contains_key(&prefix) {
                return Err(error(format!(
                    "A file outside the checkpoint blocks restore: {}",
                    prefix.display()
                )));
            }
            if prefix == *name && meta.is_dir() {
                let mut folders = vec![root.join(name)];
                while let Some(folder) = folders.pop() {
                    let mut children = tokio::fs::read_dir(folder).await?;
                    while let Some(child) = children.next_entry().await? {
                        if child.file_type().await?.is_dir() {
                            folders.push(child.path());
                        } else if !child
                            .path()
                            .strip_prefix(root)
                            .is_ok_and(|p| known.contains_key(p))
                        {
                            return Err(error(format!(
                                "A file outside the checkpoint blocks restore: {}",
                                child.path().display()
                            )));
                        }
                    }
                }
            }
            if !meta.is_dir() {
                break;
            }
        }
    }
    Ok(())
}

async fn apply(
    root: &Path,
    store: &Path,
    known: &Manifest,
    target: &Manifest,
    tree: &str,
) -> Result<(), EngineError> {
    let id = uuid::Uuid::new_v4().to_string();
    let index = store.join(format!("restore-{id}.index"));
    let staging = store.join(format!("restore-{id}"));
    tokio::fs::create_dir(&staging).await?;
    git(root, Some(store), Some(&index), &["read-tree", tree], None).await?;
    let prefix = format!("{}/", staging.display());
    git(
        root,
        Some(store),
        Some(&index),
        &[
            "checkout-index",
            "--all",
            "--force",
            &format!("--prefix={prefix}"),
        ],
        None,
    )
    .await?;
    let result = async {
        for name in known.keys() {
            // The recovery tree preserves every removed byte, including
            // untracked files and symlinks. Ignored paths never enter known.
            match tokio::fs::symlink_metadata(root.join(name)).await {
                Ok(meta) if !meta.is_dir() => {
                    tokio::fs::remove_file(root.join(name)).await?;
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(EngineError::Io(e)),
            }
        }
        for name in target.keys() {
            let source = staging.join(name);
            let dest = root.join(name);
            if tokio::fs::symlink_metadata(&dest)
                .await
                .is_ok_and(|m| m.is_dir())
            {
                let mut folders = vec![dest.clone()];
                let mut empty = Vec::new();
                while let Some(folder) = folders.pop() {
                    let mut children = tokio::fs::read_dir(&folder).await?;
                    while let Some(child) = children.next_entry().await? {
                        if child.file_type().await?.is_dir() {
                            folders.push(child.path());
                        }
                    }
                    empty.push(folder);
                }
                for folder in empty.into_iter().rev() {
                    tokio::fs::remove_dir(folder).await?;
                }
            }
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let meta = tokio::fs::symlink_metadata(&source).await?;
            if meta.file_type().is_symlink() {
                let target = tokio::fs::read_link(&source).await?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, &dest)?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(target, &dest)?;
            } else {
                tokio::fs::copy(source, dest).await?;
            }
        }
        Ok(())
    }
    .await;
    // Only generated materializations are removed. Durable trees and refs stay.
    let _ = tokio::fs::remove_file(index).await;
    let _ = tokio::fs::remove_dir_all(staging).await;
    result
}

pub(super) async fn restore(
    root: &Path,
    store: &Path,
    current: &str,
    target: &str,
) -> Result<(), EngineError> {
    let before = manifest(root, store, current).await?;
    let target_manifest = manifest(root, store, target).await?;
    preflight(root, &before, &target_manifest).await?;
    if let Err(failure) = apply(root, store, &before, &target_manifest, target).await {
        let mut known = target_manifest;
        known.extend(before.clone());
        // This is also safe when preflight refused: the checkout still has
        // its original state and no recovery writes are needed.
        if let Err(recovery) = apply(root, store, &known, &before, current).await {
            return Err(error(format!(
                "{failure}; automatic recovery failed: {recovery}. Restore the preserved recovery checkpoint."
            )));
        }
        return Err(failure);
    }
    Ok(())
}

pub(super) async fn validate_restore(
    root: &Path,
    store: &Path,
    before: &str,
    target: &str,
) -> Result<(), EngineError> {
    preflight(
        root,
        &manifest(root, store, before).await?,
        &manifest(root, store, target).await?,
    )
    .await
}

pub(super) async fn recover_interrupted(
    root: &Path,
    store: &Path,
    before: &str,
    target: &str,
) -> Result<(), EngineError> {
    let before_manifest = manifest(root, store, before).await?;
    let mut known = manifest(root, store, target).await?;
    known.extend(before_manifest.clone());
    preflight(root, &known, &before_manifest).await?;
    apply(root, store, &known, &before_manifest, before).await
}
