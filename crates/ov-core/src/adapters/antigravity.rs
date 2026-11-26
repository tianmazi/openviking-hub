use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::fs_util::{atomic_write, backup_file};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub struct AntigravityAdapter;

const PROJECT_RULE_BLOCK: &str = "\n<!-- openviking-desktop:start -->\n## OpenViking Shared Memory Protocol\nBefore starting complex tasks, call `openviking_recall` to retrieve relevant user preferences and project context. After completing a milestone or architectural decision, call `openviking_remember` to persist durable takeaways.\n<!-- openviking-desktop:end -->\n";

impl AntigravityAdapter {
    /// Global MCP config shared by Antigravity 2.0 Desktop App, IDE, and CLI.
    /// Never writes to deprecated `~/.gemini/antigravity/`.
    fn mcp_config_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".gemini").join("config").join("mcp_config.json")
    }

    /// Global Hooks config supported by Antigravity 2.0 Desktop App, IDE, and CLI.
    fn hooks_config_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".gemini").join("config").join("hooks.json")
    }

    fn detect_desktop_app() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            for app in [
                "/Applications/Antigravity.app",
                "/Applications/Antigravity IDE.app",
            ] {
                let p = PathBuf::from(app);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                for rel in [
                    "Programs\\Antigravity\\Antigravity.exe",
                    "Antigravity\\Antigravity.exe",
                    "agy\\bin\\agy.exe",
                ] {
                    let p = PathBuf::from(&local).join(rel);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            for path in [
                "/usr/bin/antigravity",
                "/opt/Antigravity/antigravity",
                "/usr/local/bin/antigravity",
            ] {
                let p = PathBuf::from(path);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        fs::read_to_string(Self::mcp_config_path(paths))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.get("mcpServers")?.get("openviking").cloned())
            .is_some()
    }
}

impl AgentAdapter for AntigravityAdapter {
    fn id(&self) -> AgentId {
        AgentId::Antigravity
    }

    fn detect(&self, paths: &PlatformPaths) -> AgentStatus {
        let desktop_app = Self::detect_desktop_app();
        let cli_bin = paths
            .resolve_binary("antigravity")
            .or_else(|| paths.resolve_binary("agy"));
        let config_dir = paths.home.join(".gemini").join("config");
        let installed = desktop_app.is_some() || cli_bin.is_some() || config_dir.exists();
        let version = cli_bin
            .as_deref()
            .filter(|p| p.is_file())
            .and_then(probe_cli_version);
        let connected = Self::is_connected(paths);

        let mut detected_surfaces = Vec::new();
        if desktop_app.is_some() {
            detected_surfaces.push("Desktop App".to_string());
        }
        if cli_bin.is_some() {
            detected_surfaces.push("CLI".to_string());
        }
        if detected_surfaces.is_empty() && config_dir.exists() {
            detected_surfaces.push("Desktop / IDE".to_string());
        }

        AgentStatus {
            id: AgentId::Antigravity.as_str().to_string(),
            agent_id: AgentId::Antigravity,
            name: "Antigravity 反重力 (Desktop + CLI)".to_string(),
            integration_mode: "MCP + Global Hooks (hooks.json)".to_string(),
            installed,
            connected,
            detected_surfaces,
            binary_path: desktop_app.or(cli_bin),
            config_path: Some(Self::mcp_config_path(paths)),
            version,
            warning: Some(
                "已同步注入全局 mcp_config.json 与 hooks.json（Desktop App / IDE / CLI 共用）。接入后请在 Antigravity App 的 Customizations → Installed → MCP Servers 中点击 Refresh 生效。"
                    .to_string(),
            ),
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let bridge = opts.bridge_binary.to_string_lossy().to_string();

        // 1. Merge global MCP config into ~/.gemini/config/mcp_config.json
        let mcp_path = Self::mcp_config_path(paths);
        backup_file("antigravity", &mcp_path).map_err(|e| e.to_string())?;
        let mut root: Value = if mcp_path.exists() {
            let raw = fs::read_to_string(&mcp_path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };
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
                "command": bridge,
                "args": ["mcp", "--agent", "antigravity"],
                "disabled": false,
                "timeoutSeconds": 60
            }),
        );
        let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        atomic_write(&mcp_path, serialized.as_bytes(), false).map_err(|e| e.to_string())?;

        // 2. Merge global Hooks into ~/.gemini/config/hooks.json (Antigravity 2.0 standard)
        let hooks_path = Self::hooks_config_path(paths);
        backup_file("antigravity", &hooks_path).map_err(|e| e.to_string())?;
        let mut hooks_root: Value = if hooks_path.exists() {
            let raw = fs::read_to_string(&hooks_path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };
        if !hooks_root.is_object() {
            hooks_root = json!({});
        }
        let hooks_obj = hooks_root.as_object_mut().unwrap();
        for (event_name, cli_event) in [
            ("SessionStart", "session-start"),
            ("UserPromptSubmit", "user-prompt-submit"),
            ("Stop", "stop"),
        ] {
            let cmd = format!("\"{}\" hook {} --agent antigravity", bridge, cli_event);
            hooks_obj.insert(
                event_name.to_string(),
                json!([
                    {
                        "matcher": "",
                        "hooks": [
                            {
                                "type": "command",
                                "command": cmd
                            }
                        ]
                    }
                ]),
            );
        }
        let hooks_str = serde_json::to_string_pretty(&hooks_root).map_err(|e| e.to_string())?;
        atomic_write(&hooks_path, hooks_str.as_bytes(), false).map_err(|e| e.to_string())?;

        // 3. Optional project rule block
        if opts.write_project_rules {
            if let Some(proj) = &opts.project_dir {
                let rule_file = proj.join(".agents").join("rules").join("openviking-memory.md");
                let existing = fs::read_to_string(&rule_file).unwrap_or_default();
                if !existing.contains("openviking-desktop:start") {
                    let updated = format!("{}{}", existing, PROJECT_RULE_BLOCK);
                    atomic_write(&rule_file, updated.as_bytes(), false)
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        let mcp_path = Self::mcp_config_path(paths);
        if mcp_path.exists() {
            backup_file("antigravity", &mcp_path).map_err(|e| e.to_string())?;
            if let Ok(raw) = fs::read_to_string(&mcp_path) {
                if let Ok(mut root) = serde_json::from_str::<Value>(&raw) {
                    if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut())
                    {
                        servers.remove("openviking");
                    }
                    let out = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
                    atomic_write(&mcp_path, out.as_bytes(), false).map_err(|e| e.to_string())?;
                }
            }
        }

        let hooks_path = Self::hooks_config_path(paths);
        if hooks_path.exists() {
            backup_file("antigravity", &hooks_path).map_err(|e| e.to_string())?;
            if let Ok(raw) = fs::read_to_string(&hooks_path) {
                if let Ok(mut root) = serde_json::from_str::<Value>(&raw) {
                    if let Some(obj) = root.as_object_mut() {
                        for key in ["SessionStart", "UserPromptSubmit", "Stop"] {
                            if let Some(val) = obj.get(key) {
                                if val.to_string().contains("ov-bridge") {
                                    obj.remove(key);
                                }
                            }
                        }
                    }
                    let out = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
                    atomic_write(&hooks_path, out.as_bytes(), false).map_err(|e| e.to_string())?;
                }
            }
        }

        Ok(self.detect(paths))
    }

    fn scan_legacy_memories(&self, paths: &PlatformPaths) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let gemini_md = paths.home.join(".gemini").join("GEMINI.md");
        if gemini_md.is_file() {
            files.push(gemini_md);
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_antigravity_mcp_and_hooks_roundtrip() {
        let tmp = tempdir().unwrap();
        let paths = PlatformPaths {
            home: tmp.path().to_path_buf(),
        };
        let adapter = AntigravityAdapter;
        let opts = ConnectOptions {
            bridge_binary: PathBuf::from("/usr/local/bin/ov-bridge"),
            write_project_rules: false,
            project_dir: None,
        };
        let status = adapter.connect(&paths, &opts).unwrap();
        assert!(status.connected);
        let hooks = fs::read_to_string(paths.home.join(".gemini/config/hooks.json")).unwrap();
        assert!(hooks.contains("ov-bridge"));
        assert!(hooks.contains("hook session-start --agent antigravity"));

        let after = adapter.disconnect(&paths).unwrap();
        assert!(!after.connected);
    }
}
