use super::claude_code::{edit_mcp_server, lock_configs, write_configs, ConfigFile};
use super::codex::{read_toml, set_mcp_server};
use serde_json::json;
use std::path::Path;

/// Inject OpenViking MCP configuration into a WSL distribution's Linux home directory
/// via UNC path, strictly using LF line endings.
pub fn inject_wsl_home(
    distro_name: &str,
    wsl_home: &Path,
    linux_bridge_path: &str,
) -> Result<Vec<String>, String> {
    let home = wsl_home.to_string_lossy().replace('\\', "/");
    let linux_home = home.strip_prefix("//").and_then(|unc| {
        let (host, share) = unc.split_once('/')?;
        if !host.eq_ignore_ascii_case("wsl$") && !host.eq_ignore_ascii_case("wsl.localhost") {
            return None;
        }
        let (distro, home) = share.split_once('/')?;
        distro.eq_ignore_ascii_case(distro_name)
            .then(|| format!("/{}", home.trim_end_matches('/')))
    });
    let bridge = resolve_bridge(wsl_home, linux_home.as_deref(), linux_bridge_path)?;
    let backup_ns = format!("wsl-{distro_name}");
    let _lock = lock_configs()?;

    let claude = ConfigFile::read(wsl_home.join(".claude.json"))?;
    let codex = ConfigFile::read(wsl_home.join(".codex/config.toml"))?;
    let claude_content;
    let codex_content;
    let mut updated_targets = Vec::new();
    let mut edits = Vec::new();

    if claude.original.is_some() || wsl_home.join(".claude").is_dir() {
        claude_content = edit_mcp_server(&claude, Some(json!({
            "command": bridge,
            "args": ["mcp", "--agent", "claude-code"],
            "env": {}
        })))?;
        edits.push((&claude, claude_content.as_deref()));
        updated_targets.push("Claude Code (WSL)".to_string());
    }
    if codex.original.is_some() || wsl_home.join(".codex").is_dir() {
        let mut doc = read_toml(&codex)?;
        set_mcp_server(&codex, &mut doc, &bridge)?;
        codex_content = doc.to_string().replace("\r\n", "\n");
        edits.push((&codex, Some(codex_content.as_str())));
        updated_targets.push("Codex CLI (WSL)".to_string());
    }

    write_configs(&backup_ns, &edits)?;
    Ok(updated_targets)
}

fn resolve_bridge(wsl_home: &Path, linux_home: Option<&str>, path: &str) -> Result<String, String> {
    let path = if let Some(relative) = path.strip_prefix("~/") {
        let home = linux_home.ok_or("Cannot expand ~/ from this WSL home. Use an absolute Linux bridge path.")?;
        format!("{home}/{relative}")
    } else if path.starts_with('/') && !path.contains('\\') {
        path.to_string()
    } else {
        return Err("Use an absolute Linux path or ~/ for the WSL ov-bridge binary.".to_string());
    };

    let parts = path.split('/').filter(|part| !part.is_empty());
    let mut normalized = String::new();
    for part in parts {
        if matches!(part, "." | "..") || part.contains(['\\', ':']) {
            return Err(format!("Invalid Linux ov-bridge path: {path}"));
        }
        normalized.push('/');
        normalized.push_str(part);
    }
    if normalized.is_empty() {
        return Err(format!("Invalid Linux ov-bridge path: {path}"));
    }

    if let Some(home) = linux_home {
        if let Some(relative) = normalized.strip_prefix(home)
            .and_then(|suffix| suffix.strip_prefix('/').or_else(|| suffix.is_empty().then_some("")))
        {
            if !wsl_home.join(relative).is_file() {
                return Err(format!(
                    "Linux ov-bridge binary not found at {normalized}. Install the Linux ov-bridge binary into WSL first."
                ));
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_wsl_lf_injection() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::create_dir_all(dir.path().join(".codex")).unwrap();
        let claude_path = dir.path().join(".claude.json");
        fs::write(&claude_path, "{\"mcpServers\": {\"other\": {\"command\": \"other\"}}}\r\n").unwrap();
        let updated =
            inject_wsl_home("Ubuntu", dir.path(), "/usr/local/bin/ov-bridge").unwrap();
        assert_eq!(updated.len(), 2);

        let claude_content = fs::read_to_string(claude_path).unwrap();
        assert!(!claude_content.contains("\r\n"));
        let config: serde_json::Value = serde_json::from_str(&claude_content).unwrap();
        assert_eq!(config["mcpServers"]["openviking"]["command"], "/usr/local/bin/ov-bridge");
        assert_eq!(config["mcpServers"]["other"]["command"], "other");

        let codex_content =
            fs::read_to_string(dir.path().join(".codex").join("config.toml")).unwrap();
        assert!(!codex_content.contains("\r\n"));
        assert!(codex_content.contains("[mcp_servers.openviking]"));
    }

    #[test]
    fn home_bridge_must_exist_and_tilde_is_expanded() {
        let dir = tempdir().unwrap();
        let home = Some("/home/dev");
        let error = resolve_bridge(dir.path(), home, "~/.openviking/bin/ov-bridge").unwrap_err();
        assert!(error.contains("Install the Linux ov-bridge binary into WSL first"));
        let bin = dir.path().join(".openviking/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("ov-bridge"), "").unwrap();
        assert_eq!(
            resolve_bridge(dir.path(), home, "~/.openviking/bin/ov-bridge").unwrap(),
            "/home/dev/.openviking/bin/ov-bridge"
        );
        assert!(resolve_bridge(dir.path(), home, "~dev/bin/ov-bridge").is_err());
    }

    #[test]
    fn malformed_wsl_config_leaves_other_config_untouched() {
        let dir = tempdir().unwrap();
        let claude = dir.path().join(".claude.json");
        let codex = dir.path().join(".codex/config.toml");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        for (json, toml, invalid) in [
            ("{broken", "# User config\n", &claude),
            (r#"{"mcpServers":{"user":{"command":"user"}}}"#, "[broken", &codex),
        ] {
            fs::write(&claude, json).unwrap();
            fs::write(&codex, toml).unwrap();
            let error = inject_wsl_home("Ubuntu", dir.path(), "/usr/local/bin/ov-bridge").unwrap_err();
            assert!(error.contains(&invalid.display().to_string()));
            assert_eq!(fs::read_to_string(&claude).unwrap(), json);
            assert_eq!(fs::read_to_string(&codex).unwrap(), toml);
        }
    }
}
