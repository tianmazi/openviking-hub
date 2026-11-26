use crate::fs_util::{atomic_write, backup_file};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use toml_edit::{value, Array, DocumentMut, Item, Table};

/// Inject OpenViking MCP configuration into a WSL distribution's Linux home directory
/// via UNC path (`\\wsl$\<Distro>\home\<user>`), strictly using LF line endings.
pub fn inject_wsl_home(
    distro_name: &str,
    wsl_home: &Path,
    linux_bridge_path: &str,
) -> Result<Vec<String>, String> {
    let mut updated_targets = Vec::new();
    let backup_ns = format!("wsl-{}", distro_name);

    // 1. Claude Code (~/.claude.json)
    let claude_json = wsl_home.join(".claude.json");
    if claude_json.exists() || wsl_home.join(".claude").exists() {
        backup_file(&backup_ns, &claude_json).map_err(|e| e.to_string())?;
        let mut root: Value = fs::read_to_string(&claude_json)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}));
        if !root.is_object() {
            root = json!({});
        }
        let servers = root
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| json!({}));
        if !servers.is_object() {
            *servers = json!({});
        }
        servers.as_object_mut().unwrap().insert(
            "openviking".to_string(),
            json!({
                "command": linux_bridge_path,
                "args": ["mcp", "--agent", "claude-code"],
                "env": {}
            }),
        );
        let lf_json = serde_json::to_string_pretty(&root)
            .map_err(|e| e.to_string())?
            .replace("\r\n", "\n");
        atomic_write(&claude_json, lf_json.as_bytes(), false).map_err(|e| e.to_string())?;
        updated_targets.push("Claude Code (WSL)".to_string());
    }

    // 2. Codex CLI (~/.codex/config.toml)
    let codex_toml = wsl_home.join(".codex").join("config.toml");
    if codex_toml.exists() || wsl_home.join(".codex").exists() {
        backup_file(&backup_ns, &codex_toml).map_err(|e| e.to_string())?;
        let content = fs::read_to_string(&codex_toml).unwrap_or_default();
        let mut doc = content
            .parse::<DocumentMut>()
            .map_err(|e| e.to_string())?;
        let mcp_servers = doc
            .entry("mcp_servers")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or("mcp_servers in WSL config.toml is not a table")?;
        let mut ov_table = Table::new();
        ov_table["command"] = value(linux_bridge_path);
        let mut args = Array::new();
        args.push("mcp");
        args.push("--agent");
        args.push("codex");
        ov_table["args"] = value(args);
        mcp_servers.insert("openviking", Item::Table(ov_table));
        let lf_toml = doc.to_string().replace("\r\n", "\n");
        atomic_write(&codex_toml, lf_toml.as_bytes(), false).map_err(|e| e.to_string())?;
        updated_targets.push("Codex CLI (WSL)".to_string());
    }

    Ok(updated_targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_wsl_lf_injection() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::create_dir_all(dir.path().join(".codex")).unwrap();
        let updated =
            inject_wsl_home("Ubuntu", dir.path(), "/home/dev/.openviking/bin/ov-bridge").unwrap();
        assert_eq!(updated.len(), 2);

        let claude_content = fs::read_to_string(dir.path().join(".claude.json")).unwrap();
        assert!(!claude_content.contains("\r\n"));
        assert!(claude_content.contains("ov-bridge"));

        let codex_content =
            fs::read_to_string(dir.path().join(".codex").join("config.toml")).unwrap();
        assert!(!codex_content.contains("\r\n"));
        assert!(codex_content.contains("[mcp_servers.openviking]"));
    }
}
