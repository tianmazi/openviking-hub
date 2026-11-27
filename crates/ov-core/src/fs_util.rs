use chrono::Utc;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum FsUtilError {
    #[error("invalid parent directory for target path")]
    InvalidParent,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to persist atomic file {path}: {reason}")]
    Persist { path: PathBuf, reason: String },
}

/// Cross-platform atomic file write with short retry backoff on Windows
/// (protects against transient antivirus/IDE file locks and running-binary replacement)
/// and optional 0o600 permissions on Unix.
pub fn atomic_write(target: &Path, content: &[u8], secret_mode: bool) -> Result<(), FsUtilError> {
    atomic_write_with_mode(target, content, secret_mode.then_some(0o600))
}

pub(crate) fn atomic_write_with_mode(
    target: &Path,
    content: &[u8],
    mode: Option<u32>,
) -> Result<(), FsUtilError> {
    let parent = target.parent().ok_or(FsUtilError::InvalidParent)?;
    fs::create_dir_all(parent)?;

    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(content)?;

    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file().set_permissions(fs::Permissions::from_mode(mode))?;
    }

    #[cfg(not(unix))]
    let _ = mode;

    tmp.as_file().sync_all()?;

    let mut current_tmp = Some(tmp);
    for attempt in 0..4 {
        let file = current_tmp.take().unwrap();
        match file.persist(target) {
            Ok(_) => return Ok(()),
            Err(err) => {
                // On Windows, replacing a running executable (e.g. `ov-bridge.exe` held open
                // by an active MCP client) or a read-only file returns ERROR_ACCESS_DENIED (os error 5).
                // Clear read-only flags and try the Windows rename-aside swap (`target` -> `target.<id>.old`).
                #[cfg(windows)]
                if err.error.kind() == std::io::ErrorKind::PermissionDenied && target.is_file() {
                    if let Ok(meta) = fs::metadata(target) {
                        let mut perms = meta.permissions();
                        if perms.readonly() {
                            perms.set_readonly(false);
                            let _ = fs::set_permissions(target, perms);
                        }
                    }
                    let file_name = target
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file");
                    let stale = parent.join(format!("{file_name}.{}.old", Uuid::new_v4().simple()));
                    if fs::rename(target, &stale).is_ok() {
                        match err.file.persist(target) {
                            Ok(_) => {
                                let _ = fs::remove_file(&stale);
                                return Ok(());
                            }
                            Err(retry_err) => {
                                let _ = fs::rename(&stale, target);
                                if attempt == 3 {
                                    return Err(FsUtilError::Persist {
                                        path: target.to_path_buf(),
                                        reason: retry_err.error.to_string(),
                                    });
                                }
                                current_tmp = Some(retry_err.file);
                                thread::sleep(Duration::from_millis(60 * (attempt as u64 + 1)));
                                continue;
                            }
                        }
                    }
                }

                if attempt == 3 {
                    return Err(FsUtilError::Persist {
                        path: target.to_path_buf(),
                        reason: err.error.to_string(),
                    });
                }
                current_tmp = Some(err.file);
                thread::sleep(Duration::from_millis(60 * (attempt as u64 + 1)));
            }
        }
    }
    Ok(())
}

/// Remove leftover `*.old` files created during Windows running-binary swaps (best-effort).
pub(crate) fn cleanup_stale_old_files(dir: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with(prefix) && name.ends_with(".old") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// Create a timestamped backup under `~/.openviking/backups/<agent_id>/` before mutating any config file.
pub fn backup_file(agent_id: &str, source: &Path) -> Result<Option<PathBuf>, FsUtilError> {
    if !source.exists() {
        return Ok(None);
    }
    let home = dirs::home_dir().ok_or(FsUtilError::InvalidParent)?;
    let backup_dir = home.join(".openviking").join("backups").join(agent_id);
    backup_into(&backup_dir, source).map(Some)
}

fn backup_into(backup_dir: &Path, source: &Path) -> Result<PathBuf, FsUtilError> {
    fs::create_dir_all(backup_dir)?;
    let file_name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ_%f");
    let backup_path = backup_dir.join(format!("{file_name}.{ts}_{}.bak", Uuid::new_v4().simple()));
    fs::copy(source, &backup_path)?;
    Ok(backup_path)
}

/// Find the most recent backup for a given agent and original file name.
pub fn latest_backup(agent_id: &str, original_file_name: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let backup_dir = home.join(".openviking").join("backups").join(agent_id);
    let prefix = format!("{}.", original_file_name);
    let mut matches: Vec<PathBuf> = fs::read_dir(backup_dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with(&prefix) && s.ends_with(".bak"))
                .unwrap_or(false)
        })
        .collect();
    matches.sort();
    matches.pop()
}

/// Compute lowercase hex SHA-256 hash of UTF-8 text (used for optimistic concurrency in Dreaming).
pub fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_atomic_write_and_hash() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("test.toml");
        atomic_write(&path, b"hello = \"world\"\n", true).unwrap();
        let read = fs::read_to_string(&path).unwrap();
        assert_eq!(read, "hello = \"world\"\n");
        assert_eq!(sha256_hex(&read).len(), 64);
    }

    #[test]
    fn test_cleanup_stale_old_files() {
        let dir = tempdir().unwrap();
        let stale = dir.path().join("ov-bridge.exe.abc123.old");
        let keep = dir.path().join("ov-bridge.exe");
        fs::write(&stale, "old").unwrap();
        fs::write(&keep, "live").unwrap();
        cleanup_stale_old_files(dir.path(), "ov-bridge.exe.");
        assert!(!stale.exists());
        assert!(keep.exists());
    }

    #[test]
    fn backups_keep_both_versions() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("config");
        let backups = dir.path().join("backups");
        fs::write(&source, "first").unwrap();
        let first = backup_into(&backups, &source).unwrap();
        fs::write(&source, "second").unwrap();
        let second = backup_into(&backups, &source).unwrap();

        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(first).unwrap(), "first");
        assert_eq!(fs::read_to_string(second).unwrap(), "second");
    }
}
