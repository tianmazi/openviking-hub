use super::claude_code::{
    edit_grouped_hooks, is_managed_hook, lock_configs, write_configs, ConfigFile, HOOK_EVENTS,
};
use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::Value;
use std::env;
#[cfg(target_os = "windows")]
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

    fn hooks_path(paths: &PlatformPaths) -> PathBuf {
        Self::codex_home(paths).join("hooks.json")
    }

    #[cfg(target_os = "windows")]
    fn desktop_cli(paths: &PlatformPaths) -> Option<PathBuf> {
        let config = ConfigFile::read(Self::config_path(paths)).ok();
        let doc = config.as_ref().and_then(|file| read_toml(file).ok());
        let configured = env::var_os("CODEX_CLI_PATH")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .or_else(|| {
                let doc = doc.as_ref()?;
                let path = doc.get("CODEX_CLI_PATH")
                    .or_else(|| doc.get("shell_environment_policy")?.get("set")?.get("CODEX_CLI_PATH"))?
                    .as_str()?;
                Some(paths.expand_tilde(std::path::Path::new(path)))
            });
        if let Some(path) = configured.filter(|path| path.is_file()) {
            return Some(path);
        }

        let bin = PathBuf::from(env::var_os("LOCALAPPDATA")?).join("OpenAI/Codex/bin");
        let candidates = fs::read_dir(&bin).into_iter().flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("codex.exe"));
        std::iter::once(bin.join("codex.exe")).chain(candidates)
            .filter_map(|path| {
                let metadata = path.metadata().ok()?;
                metadata.is_file().then(|| (metadata.modified().ok(), path))
            })
            .max()
            .map(|(_, path)| path)
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

    fn is_connected(doc: Option<&DocumentMut>, hooks: &ConfigFile) -> bool {
        let has_mcp = doc.and_then(|doc| doc.get("mcp_servers"))
            .and_then(Item::as_table_like)
            .is_some_and(|servers| servers.contains_key("openviking"));
        if !has_mcp {
            return false;
        }
        let Ok(root) = hooks.json() else {
            return false;
        };
        let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
            return false;
        };
        HOOK_EVENTS.into_iter().all(|(event, cli_event)| {
            hooks.get(event).and_then(Value::as_array).into_iter().flatten().any(|group| {
                group.get("hooks").and_then(Value::as_array).is_some_and(|handlers| {
                    handlers.iter().any(|handler| is_managed_hook(handler, cli_event, AgentId::Codex))
                })
            })
        })
    }

    fn edit_configs(home: PathBuf, bridge: Option<&str>) -> Result<(), String> {
        let _lock = lock_configs()?;
        let config = ConfigFile::read(home.join("config.toml"))?;
        let hooks = ConfigFile::read(home.join("hooks.json"))?;
        let mut doc = read_toml(&config)?;
        if let Some(bridge) = bridge {
            set_mcp_server(&config, &mut doc, bridge)?;
            let features = doc.entry("features").or_insert(Item::Table(Table::new()));
            let features = features.as_table_like_mut()
                .ok_or_else(|| config.invalid("features must be a table"))?;
            match features.get("hooks") {
                None => {
                    features.insert("hooks", value(true));
                }
                Some(hooks) if hooks.as_bool().is_none() => {
                    return Err(config.invalid("features.hooks must be a boolean"));
                }
                Some(_) => {}
            }
        } else if let Some(servers) = doc.get_mut("mcp_servers") {
            servers.as_table_like_mut()
                .ok_or_else(|| config.invalid("mcp_servers must be a table"))?
                .remove("openviking");
        }
        let hooks_content = edit_grouped_hooks(&hooks, AgentId::Codex, bridge)?;
        let config_content = (bridge.is_some() || config.original.is_some()).then(|| doc.to_string());
        write_configs("codex", &[(&config, config_content.as_deref()), (&hooks, hooks_content.as_deref())])
    }
}

pub(super) fn read_toml(file: &ConfigFile) -> Result<DocumentMut, String> {
    file.original
        .as_deref()
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|e| file.invalid(e))
}

pub(super) fn set_mcp_server(file: &ConfigFile, doc: &mut DocumentMut, bridge: &str) -> Result<(), String> {
    let servers = doc.entry("mcp_servers").or_insert(Item::Table(Table::new()));
    let servers = servers.as_table_like_mut()
        .ok_or_else(|| file.invalid("mcp_servers must be a table"))?;
    let mut server = Table::new();
    server["command"] = value(bridge);
    let mut args = Array::new();
    args.push("mcp");
    args.push("--agent");
    args.push("codex");
    server["args"] = value(args);
    servers.insert("openviking", Item::Table(server));
    Ok(())
}

impl AgentAdapter for CodexAdapter {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn detect(&self, paths: &PlatformPaths) -> AgentStatus {
        let desktop_app = Self::detect_desktop_app();
        let cli_bin = paths.resolve_binary("codex");
        #[cfg(target_os = "windows")]
        let (desktop_app, cli_bin) = {
            let bundled_cli = Self::desktop_cli(paths);
            (desktop_app.or_else(|| bundled_cli.clone()), bundled_cli.or(cli_bin))
        };
        let dir_exists = Self::codex_home(paths).exists();
        let installed = cli_bin.is_some() || desktop_app.is_some() || dir_exists;
        let version = cli_bin.as_deref().and_then(probe_cli_version);
        let doc = ConfigFile::read(Self::config_path(paths)).ok().and_then(|file| read_toml(&file).ok());
        let connected = ConfigFile::read(Self::hooks_path(paths)).ok()
            .is_some_and(|hooks| Self::is_connected(doc.as_ref(), &hooks));
        let hooks_disabled = doc.as_ref().and_then(|doc| doc.get("features"))
            .and_then(|features| features.get("hooks")).and_then(Item::as_bool) == Some(false);

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
            integration_mode: "MCP + Hooks (config.toml + hooks.json)".to_string(),
            installed,
            connected,
            detected_surfaces,
            binary_path: desktop_app.or(cli_bin),
            config_path: Some(Self::config_path(paths)),
            version,
            warning: Some(if hooks_disabled {
                "features.hooks = false in config.toml disables Codex lifecycle hooks.".to_string()
            } else {
                "Codex Desktop 与 CLI 共用用户级 config.toml（桌面端会忽略项目级配置）。注入后请重启 Codex Desktop 并新建 Thread，CLI 首次启动若提示 \"Hooks need review\" 请选择 Trust all and continue。".to_string()
            }),
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let bridge = opts.bridge_binary.to_string_lossy();
        Self::edit_configs(Self::codex_home(paths), Some(&bridge))?;
        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        Self::edit_configs(Self::codex_home(paths), None)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_preserves_disabled_hooks_comments_and_user_entries() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        let config = home.join("config.toml");
        let hooks = home.join("hooks.json");
        fs::write(&config, "# User comment\n[features]\nhooks = false\n\n[mcp_servers.user]\ncommand = 'user'\n").unwrap();
        let user_group = json!({ "matcher": "", "hooks": [{ "type": "command", "command": "ov-bridge status" }] });
        let original_hooks = json!({ "hooks": {
            "PreToolUse": [{ "hooks": [{ "type": "command", "command": "ov-bridge hook stop --agent codex" }] }],
            "Stop": [user_group]
        } });
        fs::write(&hooks, original_hooks.to_string()).unwrap();

        for _ in 0..2 {
            CodexAdapter::edit_configs(home.to_path_buf(), Some("/test/ov-bridge")).unwrap();
        }
        let config_file = ConfigFile::read(config.clone()).unwrap();
        let doc = read_toml(&config_file).unwrap();
        assert_eq!(doc["features"]["hooks"].as_bool(), Some(false));
        assert!(doc.to_string().contains("# User comment"));
        let hook_file = ConfigFile::read(hooks.clone()).unwrap();
        let root = hook_file.json().unwrap();
        assert!(CodexAdapter::is_connected(Some(&doc), &hook_file));
        assert_eq!(root["hooks"]["PreToolUse"], original_hooks["hooks"]["PreToolUse"]);
        assert_eq!(root["hooks"]["Stop"][0], user_group);
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(root["hooks"]["Stop"][1]["hooks"][0]["command"], "\"/test/ov-bridge\" hook stop --agent codex");

        CodexAdapter::edit_configs(home.to_path_buf(), None).unwrap();
        let doc = read_toml(&ConfigFile::read(config).unwrap()).unwrap();
        let hook_file = ConfigFile::read(hooks).unwrap();
        assert!(!CodexAdapter::is_connected(Some(&doc), &hook_file));
        assert!(doc["mcp_servers"].get("openviking").is_none());
        assert_eq!(doc["mcp_servers"]["user"]["command"].as_str(), Some("user"));
        assert_eq!(Value::Object(hook_file.json().unwrap()), original_hooks);
    }

    #[test]
    fn enables_hooks_when_feature_is_missing() {
        let tmp = tempdir().unwrap();
        CodexAdapter::edit_configs(tmp.path().to_path_buf(), Some("/test/ov-bridge")).unwrap();
        let config = ConfigFile::read(tmp.path().join("config.toml")).unwrap();
        assert_eq!(read_toml(&config).unwrap()["features"]["hooks"].as_bool(), Some(true));
    }

    #[test]
    fn invalid_toml_or_hooks_never_changes_either_file() {
        let tmp = tempdir().unwrap();
        let config = tmp.path().join("config.toml");
        let hooks = tmp.path().join("hooks.json");
        for (toml, json, invalid) in [
            ("[broken", "{}", &config),
            ("# Keep this\n", "{broken", &hooks),
            ("# Keep this\n", r#"{"hooks":{"Stop":{}}}"#, &hooks),
        ] {
            fs::write(&config, toml).unwrap();
            fs::write(&hooks, json).unwrap();
            let error = CodexAdapter::edit_configs(tmp.path().to_path_buf(), Some("/test/ov-bridge"))
                .unwrap_err();
            assert!(error.contains(&invalid.display().to_string()));
            assert_eq!(fs::read_to_string(&config).unwrap(), toml);
            assert_eq!(fs::read_to_string(&hooks).unwrap(), json);
        }
    }
}
