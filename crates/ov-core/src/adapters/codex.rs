use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::fs_util::{atomic_write, backup_file};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use std::env;
use std::fs;
use std::path::PathBuf;
use toml_edit::{value, Array, DocumentMut, Item, Table};

pub struct CodexAdapter;

impl CodexAdapter {
    /// Respect `$CODEX_HOME` if set; otherwise default to `~/.codex`.
    /// Shared by Codex Desktop App (macOS/Windows), Codex CLI, and IDE extensions.
    fn codex_home(paths: &PlatformPaths) -> PathBuf {
        if let Ok(custom) = env::var("CODEX_HOME") {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
        paths.home.join(".codex")
    }

    fn config_path(paths: &PlatformPaths) -> PathBuf {
        Self::codex_home(paths).join("config.toml")
    }

    fn detect_desktop_app() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            for app in ["/Applications/Codex.app", "/Applications/ChatGPT.app"] {
                let p = PathBuf::from(app);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(local) = env::var("LOCALAPPDATA") {
                for rel in [
                    "Programs\\Codex\\Codex.exe",
                    "Programs\\OpenAI Codex\\Codex.exe",
                    "Programs\\ChatGPT\\ChatGPT.exe",
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
            for path in ["/usr/bin/codex-desktop", "/opt/Codex/codex"] {
                let p = PathBuf::from(path);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
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
        let cli_bin = paths.resolve_binary("codex");
        let desktop_app = Self::detect_desktop_app();
        let dir_exists = Self::codex_home(paths).exists();
        let installed = cli_bin.is_some() || desktop_app.is_some() || dir_exists;
        let version = cli_bin.as_deref().and_then(probe_cli_version);
        let connected = Self::is_connected(paths);

        let mut detected_surfaces = Vec::new();
        if desktop_app.is_some() {
            detected_surfaces.push("Desktop App".to_string());
        }
        if cli_bin.is_some() {
            detected_surfaces.push("CLI".to_string());
        }
        if detected_surfaces.is_empty() && dir_exists {
            detected_surfaces.push("CLI / Desktop".to_string());
        }

        AgentStatus {
            id: AgentId::Codex.as_str().to_string(),
            agent_id: AgentId::Codex,
            name: "Codex (Desktop + CLI)".to_string(),
            integration_mode: "Global TOML MCP (Desktop + CLI)".to_string(),
            installed,
            connected,
            detected_surfaces,
            binary_path: desktop_app.or(cli_bin),
            config_path: Some(Self::config_path(paths)),
            version,
            warning: Some(
                "Codex Desktop 与 CLI 共用用户级 config.toml（桌面端会忽略项目级配置）。注入后请重启 Codex Desktop 并新建 Thread，CLI 首次启动若提示 \"Hooks need review\" 请选择 Trust all and continue。"
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
        let agents_md = Self::codex_home(paths).join("AGENTS.md");
        if agents_md.is_file() {
            files.push(agents_md);
        }
        files
    }
}
