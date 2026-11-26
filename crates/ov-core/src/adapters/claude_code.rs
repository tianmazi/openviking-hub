use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::fs_util::{atomic_write, backup_file};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    fn mcp_config_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".claude.json")
    }

    fn settings_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".claude").join("settings.json")
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        let mcp_ok = fs::read_to_string(Self::mcp_config_path(paths))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.get("mcpServers")?.get("openviking").cloned())
            .is_some();
        mcp_ok
    }
}

impl AgentAdapter for ClaudeCodeAdapter {
    fn id(&self) -> AgentId {
        AgentId::ClaudeCode
    }

    fn detect(&self, paths: &PlatformPaths) -> AgentStatus {
        let bin = paths.resolve_binary("claude");
        let dir_exists = paths.home.join(".claude").exists();
        let installed = bin.is_some() || dir_exists;
        let version = bin.as_deref().and_then(probe_cli_version);
        let connected = Self::is_connected(paths);

        AgentStatus {
            id: AgentId::ClaudeCode.as_str().to_string(),
            agent_id: AgentId::ClaudeCode,
            name: "Claude Code".to_string(),
            integration_mode: "MCP + Hooks".to_string(),
            installed,
            connected,
            binary_path: bin,
            config_path: Some(Self::mcp_config_path(paths)),
            version,
            warning: None,
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let bridge = opts.bridge_binary.to_string_lossy().to_string();

        // 1. Merge MCP entry into ~/.claude.json
        let mcp_path = Self::mcp_config_path(paths);
        backup_file("claude-code", &mcp_path).map_err(|e| e.to_string())?;
        let mut mcp_root: Value = if mcp_path.exists() {
            let raw = fs::read_to_string(&mcp_path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };
        if !mcp_root.is_object() {
            mcp_root = json!({});
        }
        let servers = mcp_root
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
                "args": ["mcp", "--agent", "claude-code"],
                "env": {}
            }),
        );
        let serialized = serde_json::to_string_pretty(&mcp_root).map_err(|e| e.to_string())?;
        atomic_write(&mcp_path, serialized.as_bytes(), false).map_err(|e| e.to_string())?;

        // 2. Merge lifecycle Hooks into ~/.claude/settings.json
        let settings_path = Self::settings_path(paths);
        backup_file("claude-code", &settings_path).map_err(|e| e.to_string())?;
        let mut settings_root: Value = if settings_path.exists() {
            let raw = fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };
        if !settings_root.is_object() {
            settings_root = json!({});
        }
        let hooks = settings_root
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| json!({}));
        if !hooks.is_object() {
            *hooks = json!({});
        }
        let hooks_obj = hooks.as_object_mut().unwrap();
        for (event_name, cli_event) in [
            ("SessionStart", "session-start"),
            ("UserPromptSubmit", "user-prompt-submit"),
            ("Stop", "stop"),
        ] {
            let cmd = format!("\"{}\" hook {} --agent claude-code", bridge, cli_event);
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
        let settings_str =
            serde_json::to_string_pretty(&settings_root).map_err(|e| e.to_string())?;
        atomic_write(&settings_path, settings_str.as_bytes(), false).map_err(|e| e.to_string())?;

        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        let mcp_path = Self::mcp_config_path(paths);
        if mcp_path.exists() {
            backup_file("claude-code", &mcp_path).map_err(|e| e.to_string())?;
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

        let settings_path = Self::settings_path(paths);
        if settings_path.exists() {
            backup_file("claude-code", &settings_path).map_err(|e| e.to_string())?;
            if let Ok(raw) = fs::read_to_string(&settings_path) {
                if let Ok(mut root) = serde_json::from_str::<Value>(&raw) {
                    if let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) {
                        for key in ["SessionStart", "UserPromptSubmit", "Stop"] {
                            if let Some(val) = hooks.get(key) {
                                if val.to_string().contains("ov-bridge") {
                                    hooks.remove(key);
                                }
                            }
                        }
                    }
                    let out = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
                    atomic_write(&settings_path, out.as_bytes(), false)
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        Ok(self.detect(paths))
    }

    fn scan_legacy_memories(&self, paths: &PlatformPaths) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let candidates = [
            paths.home.join(".claude").join("CLAUDE.md"),
            paths.home.join(".claude").join("MEMORY.md"),
        ];
        for c in candidates {
            if c.is_file() {
                files.push(c);
            }
        }
        files
    }
}
