use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::fs_util::{atomic_write, backup_file};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use std::fs;
use std::path::PathBuf;
use toml_edit::{value, Array, DocumentMut, Item, Table};

pub struct CodexAdapter;

impl CodexAdapter {
    fn config_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".codex").join("config.toml")
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        let path = Self::config_path(paths);
        let Ok(raw) = fs::read_to_string(path) else {
            return false;
        };
        let Ok(doc) = raw.parse::<DocumentMut>() else {
            return false;
        };
        doc.get("mcp_servers")
            .and_then(|i| i.as_table())
            .and_then(|t| t.get("openviking"))
            .is_some()
    }
}

impl AgentAdapter for CodexAdapter {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn detect(&self, paths: &PlatformPaths) -> AgentStatus {
        let bin = paths.resolve_binary("codex");
        let dir_exists = paths.home.join(".codex").exists();
        let installed = bin.is_some() || dir_exists;
        let version = bin.as_deref().and_then(probe_cli_version);
        let connected = Self::is_connected(paths);

        AgentStatus {
            id: AgentId::Codex.as_str().to_string(),
            agent_id: AgentId::Codex,
            name: "Codex CLI".to_string(),
            integration_mode: "MCP + Hooks (TOML)".to_string(),
            installed,
            connected,
            binary_path: bin,
            config_path: Some(Self::config_path(paths)),
            version,
            warning: Some(
                "首次启动 Codex 时会提示 \"Hooks need review\"，请在终端选择 Trust all and continue。"
                    .to_string(),
            ),
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let toml_path = Self::config_path(paths);
        backup_file("codex", &toml_path).map_err(|e| e.to_string())?;

        let content = fs::read_to_string(&toml_path).unwrap_or_default();
        let mut doc = content
            .parse::<DocumentMut>()
            .map_err(|e| format!("Failed to parse {}: {}", toml_path.display(), e))?;

        let mcp_servers = doc
            .entry("mcp_servers")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or("mcp_servers in config.toml is not a table")?;

        let mut ov_table = Table::new();
        ov_table["command"] = value(opts.bridge_binary.to_string_lossy().to_string());
        let mut args = Array::new();
        args.push("mcp");
        args.push("--agent");
        args.push("codex");
        ov_table["args"] = value(args);
        mcp_servers.insert("openviking", Item::Table(ov_table));

        atomic_write(&toml_path, doc.to_string().as_bytes(), false).map_err(|e| e.to_string())?;
        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        let toml_path = Self::config_path(paths);
        if toml_path.exists() {
            backup_file("codex", &toml_path).map_err(|e| e.to_string())?;
            let content = fs::read_to_string(&toml_path).unwrap_or_default();
            if let Ok(mut doc) = content.parse::<DocumentMut>() {
                if let Some(mcp_servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut())
                {
                    mcp_servers.remove("openviking");
                }
                atomic_write(&toml_path, doc.to_string().as_bytes(), false)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(self.detect(paths))
    }

    fn scan_legacy_memories(&self, paths: &PlatformPaths) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let agents_md = paths.home.join(".codex").join("AGENTS.md");
        if agents_md.is_file() {
            files.push(agents_md);
        }
        files
    }
}
