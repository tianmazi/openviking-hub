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
    /// Uses `~/.gemini/config/mcp_config.json` (never the deprecated `~/.gemini/antigravity/`).
    fn mcp_config_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".gemini").join("config").join("mcp_config.json")
    }

    fn detect_app_binary(paths: &PlatformPaths) -> Option<PathBuf> {
        if let Some(bin) = paths
            .resolve_binary("antigravity")
            .or_else(|| paths.resolve_binary("agy"))
        {
            return Some(bin);
        }
        #[cfg(target_os = "macos")]
        {
            let app = PathBuf::from("/Applications/Antigravity.app");
            if app.exists() {
                return Some(app);
            }
        }
        #[cfg(target_os = "linux")]
        {
            let usr = PathBuf::from("/usr/bin/antigravity");
            if usr.exists() {
                return Some(usr);
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
        let bin = Self::detect_app_binary(paths);
        let config_dir = paths.home.join(".gemini").join("config");
        let installed = bin.is_some() || config_dir.exists();
        let version = bin
            .as_deref()
            .filter(|p| p.is_file())
            .and_then(probe_cli_version);
        let connected = Self::is_connected(paths);

        AgentStatus {
            id: AgentId::Antigravity.as_str().to_string(),
            agent_id: AgentId::Antigravity,
            name: "Antigravity (反重力)".to_string(),
            integration_mode: "MCP-Only (Global JSON)".to_string(),
            installed,
            connected,
            binary_path: bin,
            config_path: Some(Self::mcp_config_path(paths)),
            version,
            warning: Some(
                "注入后请在 Antigravity IDE 的 MCP 面板点击 Refresh 生效（已避开废弃的 ~/.gemini/antigravity/ 目录）。"
                    .to_string(),
            ),
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
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

        // Uses pure Rust ov-bridge absolute path so macOS/Linux GUI apps never fail on missing PATH.
        servers.as_object_mut().unwrap().insert(
            "openviking".to_string(),
            json!({
                "command": opts.bridge_binary.to_string_lossy().to_string(),
                "args": ["mcp", "--agent", "antigravity"],
                "disabled": false,
                "timeoutSeconds": 60
            }),
        );

        let serialized = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        atomic_write(&mcp_path, serialized.as_bytes(), false).map_err(|e| e.to_string())?;

        // Optional: append memory protocol prompt to project rules for MCP-Only hosts.
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
