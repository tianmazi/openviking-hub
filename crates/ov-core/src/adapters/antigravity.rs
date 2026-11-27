use super::claude_code::{
    edit_mcp_server, hook_command, json_path_exists, lock_configs, prune_bridge_hooks,
    write_configs, ConfigFile, HOOK_EVENTS,
};
use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::json;
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

    fn edit_configs(
        paths: &PlatformPaths,
        bridge: Option<&str>,
        rule: Option<&ConfigFile>,
    ) -> Result<(), String> {
        let mcp = ConfigFile::read(Self::mcp_config_path(paths))?;
        let hooks = ConfigFile::read(Self::hooks_config_path(paths))?;
        let mcp_content = edit_mcp_server(&mcp, bridge.map(|command| json!({
            "command": command,
            "args": ["mcp", "--agent", "antigravity"],
            "disabled": false,
            "timeoutSeconds": 60
        })))?;
        let hooks_content = Self::edit_hooks(&hooks, bridge)?;
        let rule_content;
        let mut edits = vec![(&mcp, mcp_content.as_deref()), (&hooks, hooks_content.as_deref())];
        if let Some(rule) = rule {
            let existing = rule.original.as_deref().unwrap_or_default();
            if !existing.contains("openviking-desktop:start") {
                rule_content = format!("{existing}{PROJECT_RULE_BLOCK}");
                edits.push((rule, Some(rule_content.as_str())));
            }
        }
        write_configs("antigravity", &edits)
    }

    fn edit_hooks(file: &ConfigFile, bridge: Option<&str>) -> Result<Option<String>, String> {
        let mut root = file.json()?;
        let mut changed = false;
        for (event, _) in HOOK_EVENTS {
            if root.get(event).is_some_and(serde_json::Value::is_array) {
                changed |= prune_bridge_hooks(&mut root, event, file, false)?;
            }
        }
        if let Some(bridge) = bridge {
            let agent = AgentId::Antigravity;
            root.insert("openviking".to_string(), json!({
                "SessionStart": [hook_command(bridge, "session-start", agent)],
                "PreInvocation": [hook_command(bridge, "pre-invocation", agent)],
                "Stop": [hook_command(bridge, "stop", agent)]
            }));
            changed = true;
        } else {
            changed |= root.remove("openviking").is_some();
        }
        if !changed {
            return Ok(file.original.clone());
        }
        serde_json::to_string_pretty(&root).map(Some).map_err(|e| file.invalid(e))
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        json_path_exists(Self::mcp_config_path(paths), &["mcpServers", "openviking"])
            && json_path_exists(Self::hooks_config_path(paths), &["openviking"])
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
            warning: Some(if connected {
                "已同步注入全局 mcp_config.json 与 hooks.json（Desktop App / IDE / CLI 共用）。请在 Antigravity App 的 Customizations → Installed → MCP Servers 中点击 Refresh 生效。"
                    .to_string()
            } else {
                "接入后将自动注入全局 mcp_config.json 与 hooks.json（Desktop App / IDE / CLI 共用）；完成后请在 Antigravity App 的 Customizations → Installed → MCP Servers 中点击 Refresh 生效。"
                    .to_string()
            }),
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let lock = lock_configs()?;
        let rule_path = opts.project_dir.as_ref().filter(|_| opts.write_project_rules)
            .map(|proj| proj.join(".agents/rules/openviking-memory.md"));
        let rule = rule_path.map(ConfigFile::read).transpose()?;
        let bridge = opts.bridge_binary.to_string_lossy();
        Self::edit_configs(paths, Some(&bridge), rule.as_ref())?;
        drop(lock);
        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        let lock = lock_configs()?;
        Self::edit_configs(paths, None, None)?;
        drop(lock);
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
    use serde_json::Value;
    use std::fs;
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
        let path = AntigravityAdapter::hooks_config_path(&paths);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let user = json!({ "type": "command", "command": "user hook" });
        let legacy = json!({
            "user": { "Stop": [user] },
            "SessionStart": [
                { "matcher": "", "hooks": [user, { "command": "/old/ov-bridge" }] }
            ],
            "UserPromptSubmit": [{ "command": "/old/ov-bridge" }]
        });
        fs::write(&path, legacy.to_string()).unwrap();
        let mcp_path = AntigravityAdapter::mcp_config_path(&paths);
        fs::write(&mcp_path, r#"{"mcpServers":{"user":{"command":"user-mcp"}}}"#).unwrap();

        for _ in 0..2 {
            let status = adapter.connect(&paths, &opts).unwrap();
            assert!(status.connected);
            assert!(status.warning.as_deref().unwrap_or_default().contains("已同步注入"));
        }
        let read_hooks = || serde_json::from_str::<Value>(&fs::read_to_string(&path).unwrap()).unwrap();
        let retained = json!({
            "user": { "Stop": [user] },
            "SessionStart": [{ "matcher": "", "hooks": [user] }]
        });
        let mut hooks = read_hooks();
        assert_eq!(hooks["openviking"], json!({
            "SessionStart": [{ "type": "command", "command": "\"/usr/local/bin/ov-bridge\" hook session-start --agent antigravity" }],
            "PreInvocation": [{ "type": "command", "command": "\"/usr/local/bin/ov-bridge\" hook pre-invocation --agent antigravity" }],
            "Stop": [{ "type": "command", "command": "\"/usr/local/bin/ov-bridge\" hook stop --agent antigravity" }]
        }));
        hooks.as_object_mut().unwrap().remove("openviking");
        assert_eq!(hooks, retained);

        let after = adapter.disconnect(&paths).unwrap();
        assert!(!after.connected);
        assert!(after.warning.as_deref().unwrap_or_default().contains("接入后将自动注入"));
        assert_eq!(read_hooks(), retained);
        assert!(json_path_exists(mcp_path, &["mcpServers", "user"]));
    }

    #[test]
    fn invalid_hooks_leave_mcp_untouched() {
        let tmp = tempdir().unwrap();
        let paths = PlatformPaths { home: tmp.path().to_path_buf() };
        let mcp = AntigravityAdapter::mcp_config_path(&paths);
        let hooks = AntigravityAdapter::hooks_config_path(&paths);
        fs::create_dir_all(hooks.parent().unwrap()).unwrap();
        fs::write(&mcp, "{}\n").unwrap();
        fs::write(&hooks, "[]").unwrap();
        let error = AntigravityAdapter::edit_configs(&paths, Some("/test/ov-bridge"), None).unwrap_err();
        assert!(error.contains(&hooks.display().to_string()));
        assert_eq!(fs::read_to_string(mcp).unwrap(), "{}\n");
        assert_eq!(fs::read_to_string(hooks).unwrap(), "[]");
    }
}
