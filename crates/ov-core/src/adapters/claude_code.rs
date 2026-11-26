use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::fs_util::{atomic_write, backup_file};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::{json, Map, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

pub struct ClaudeCodeAdapter;

static CONFIG_EDIT_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn lock_configs() -> Result<MutexGuard<'static, ()>, String> {
    CONFIG_EDIT_LOCK.lock().map_err(|e| e.to_string())
}

pub(super) struct ConfigFile {
    pub path: PathBuf,
    pub original: Option<String>,
}

impl ConfigFile {
    pub fn read(path: PathBuf) -> Result<Self, String> {
        let original = match fs::read_to_string(&path) {
            Ok(raw) => Some(raw),
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Failed to read {}: {e}", path.display())),
        };
        Ok(Self { path, original })
    }

    pub fn invalid(&self, reason: impl std::fmt::Display) -> String {
        format!("Invalid config {}: {reason}", self.path.display())
    }

    pub fn json(&self) -> Result<Map<String, Value>, String> {
        match &self.original {
            Some(raw) => serde_json::from_str(raw).map_err(|e| self.invalid(e)),
            None => Ok(Map::new()),
        }
    }
}

// Callers prepare every edit before any write. Restore completed writes in reverse order on error.
pub(super) fn write_configs(agent: &str, edits: &[(&ConfigFile, Option<&str>)]) -> Result<(), String> {
    let edits: Vec<_> = edits.iter().copied()
        .filter(|(file, content)| file.original.as_deref() != *content)
        .collect();
    for &(file, _) in &edits {
        backup_file(agent, &file.path)
            .map_err(|e| format!("Failed to back up {}: {e}", file.path.display()))?;
    }
    for (index, &(file, content)) in edits.iter().enumerate() {
        if let Err(mut error) = write_config(file, content) {
            for &(saved, _) in edits[..index].iter().rev() {
                if let Err(e) = write_config(saved, saved.original.as_deref()) {
                    error.push_str(&format!("; rollback failed: {e}"));
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

fn write_config(file: &ConfigFile, content: Option<&str>) -> Result<(), String> {
    let result = match content {
        Some(raw) => atomic_write(&file.path, raw.as_bytes(), false).map_err(|e| e.to_string()),
        None => match fs::remove_file(&file.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        },
    };
    result.map_err(|e| format!("Failed to update {}: {e}", file.path.display()))
}

pub(super) fn edit_mcp_server(file: &ConfigFile, server: Option<Value>) -> Result<Option<String>, String> {
    let mut root = file.json()?;
    if server.is_some() {
        root.entry("mcpServers").or_insert_with(|| json!({}));
    }
    let Some(servers) = root.get_mut("mcpServers") else {
        return Ok(file.original.clone());
    };
    let servers = servers.as_object_mut().ok_or_else(|| file.invalid("mcpServers must be an object"))?;
    if let Some(server) = server {
        servers.insert("openviking".to_string(), server);
    } else if servers.remove("openviking").is_none() {
        return Ok(file.original.clone());
    }
    serde_json::to_string_pretty(&root).map(Some).map_err(|e| file.invalid(e))
}

pub(super) fn json_path_exists(file: PathBuf, keys: &[&str]) -> bool {
    let Ok(Some(raw)) = ConfigFile::read(file).map(|file| file.original) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    keys.iter().try_fold(&root, |value, &key| value.get(key)).is_some()
}

pub(super) const HOOK_EVENTS: [(&str, &str); 5] = [
    ("SessionStart", "session-start"),
    ("UserPromptSubmit", "user-prompt-submit"),
    ("Stop", "stop"),
    ("PreCompact", "pre-compact"),
    ("SessionEnd", "session-end"),
];

pub(super) fn is_managed_hook(handler: &Value, cli_event: &str, agent: AgentId) -> bool {
    let Some((program, args)) = handler
        .get("command")
        .and_then(Value::as_str)
        .and_then(|command| command.trim().rsplit_once(" hook "))
    else {
        return false;
    };
    let program = if let Some(quoted) = program.strip_prefix('"') {
        quoted.strip_suffix('"').filter(|path| !path.contains('"'))
    } else {
        Some(program).filter(|path| !path.contains(char::is_whitespace))
    };
    let executable = program.and_then(|path| path.rsplit(['/', '\\']).next());
    matches!(executable, Some("ov-bridge" | "ov-bridge.exe"))
        && args
            .strip_prefix(cli_event)
            .and_then(|args| args.strip_prefix(" --agent "))
            == Some(agent.as_str())
}

pub(super) fn prune_bridge_hooks(
    hooks: &mut Map<String, Value>,
    event: &str,
    file: &ConfigFile,
    remove_group: bool,
) -> Result<bool, String> {
    prune_hooks(hooks, event, file, remove_group, |handler| {
        handler.get("command").and_then(Value::as_str)
            .is_some_and(|command| command.contains("ov-bridge"))
    })
}

fn prune_hooks(
    hooks: &mut Map<String, Value>,
    event: &str,
    file: &ConfigFile,
    remove_group: bool,
    is_managed: impl Fn(&Value) -> bool,
) -> Result<bool, String> {
    let Some(groups) = hooks.get_mut(event) else {
        return Ok(false);
    };
    let groups = groups.as_array_mut().ok_or_else(|| file.invalid(format!("{event} must be an array")))?;
    let mut changed = false;
    groups.retain_mut(|group| {
        if is_managed(group) {
            changed = true;
            return false;
        }
        let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        if remove_group && handlers.iter().any(&is_managed) {
            changed = true;
            return false;
        }
        let before = handlers.len();
        handlers.retain(|handler| !is_managed(handler));
        let removed = handlers.len() != before;
        changed |= removed;
        !removed || !handlers.is_empty()
    });
    if groups.is_empty() && changed {
        hooks.remove(event);
    }
    Ok(changed)
}

pub(super) fn hook_command(bridge: &str, event: &str, agent: AgentId) -> Value {
    json!({ "type": "command", "command": format!("\"{bridge}\" hook {event} --agent {}", agent.as_str()) })
}

pub(super) fn edit_grouped_hooks(
    file: &ConfigFile,
    agent: AgentId,
    bridge: Option<&str>,
) -> Result<Option<String>, String> {
    let mut root = file.json()?;
    if bridge.is_some() {
        root.entry("hooks").or_insert_with(|| json!({}));
    }
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(file.original.clone());
    };
    let hooks = hooks.as_object_mut().ok_or_else(|| file.invalid("hooks must be an object"))?;
    let mut changed = false;
    for (event, cli_event) in HOOK_EVENTS {
        changed |= prune_hooks(hooks, event, file, false, |handler| {
            is_managed_hook(handler, cli_event, agent)
        })?;
        if let Some(bridge) = bridge {
            hooks.entry(event).or_insert_with(|| json!([]))
                .as_array_mut().expect("validated event array")
                .push(json!({ "matcher": "", "hooks": [hook_command(bridge, cli_event, agent)] }));
            changed = true;
        }
    }
    if !changed {
        return Ok(file.original.clone());
    }
    serde_json::to_string_pretty(&root).map(Some).map_err(|e| file.invalid(e))
}

impl ClaudeCodeAdapter {
    fn mcp_config_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".claude.json")
    }

    fn settings_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".claude").join("settings.json")
    }

    fn edit_configs(paths: &PlatformPaths, bridge: Option<&str>) -> Result<(), String> {
        let _lock = lock_configs()?;
        let mcp = ConfigFile::read(Self::mcp_config_path(paths))?;
        let settings = ConfigFile::read(Self::settings_path(paths))?;
        let mcp_content = edit_mcp_server(&mcp, bridge.map(|command| json!({
            "command": command,
            "args": ["mcp", "--agent", "claude-code"],
            "env": {}
        })))?;
        let settings_content = edit_grouped_hooks(&settings, AgentId::ClaudeCode, bridge)?;
        write_configs("claude-code", &[
            (&mcp, mcp_content.as_deref()),
            (&settings, settings_content.as_deref()),
        ])
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        json_path_exists(Self::mcp_config_path(paths), &["mcpServers", "openviking"])
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

        let mut detected_surfaces = Vec::new();
        if bin.is_some() || dir_exists {
            detected_surfaces.push("CLI".to_string());
        }

        AgentStatus {
            id: AgentId::ClaudeCode.as_str().to_string(),
            agent_id: AgentId::ClaudeCode,
            name: "Claude Code".to_string(),
            integration_mode: "MCP + Hooks".to_string(),
            installed,
            connected,
            detected_surfaces,
            binary_path: bin,
            config_path: Some(Self::mcp_config_path(paths)),
            version,
            warning: None,
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let bridge = opts.bridge_binary.to_string_lossy();
        Self::edit_configs(paths, Some(&bridge))?;
        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        Self::edit_configs(paths, None)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn hooks_roundtrip_preserves_user_groups() {
        let tmp = tempdir().unwrap();
        let paths = PlatformPaths { home: tmp.path().to_path_buf() };
        let mcp = ClaudeCodeAdapter::mcp_config_path(&paths);
        let settings = ClaudeCodeAdapter::settings_path(&paths);
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let servers = json!({ "mcpServers": { "other": { "command": "other" } } });
        fs::write(&mcp, servers.to_string()).unwrap();
        let user_group = json!({ "matcher": "", "hooks": [{ "type": "command", "command": "ov-bridge status" }] });
        let managed = hook_command("/old/ov-bridge", "user-prompt-submit", AgentId::ClaudeCode);
        let user_hooks = json!({
            "hooks": { "UserPromptSubmit": [user_group], "PreToolUse": [{ "hooks": [managed] }] }
        });
        let mut existing = user_hooks.clone();
        existing["hooks"]["UserPromptSubmit"][0]["hooks"].as_array_mut().unwrap().push(managed);
        fs::write(&settings, existing.to_string()).unwrap();

        for _ in 0..2 {
            ClaudeCodeAdapter::edit_configs(&paths, Some("/test/ov-bridge")).unwrap();
        }
        let connected = ConfigFile::read(settings.clone()).unwrap().json().unwrap();
        assert_eq!(connected["hooks"]["UserPromptSubmit"][0], user_group);
        assert_eq!(connected["hooks"]["PreToolUse"], user_hooks["hooks"]["PreToolUse"]);
        assert_eq!(connected["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
        assert!(ClaudeCodeAdapter::is_connected(&paths));

        ClaudeCodeAdapter::edit_configs(&paths, None).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&settings).unwrap()).unwrap(), user_hooks);
        assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&mcp).unwrap()).unwrap(), servers);
    }

    #[test]
    fn malformed_second_config_leaves_first_untouched() {
        let tmp = tempdir().unwrap();
        let paths = PlatformPaths { home: tmp.path().to_path_buf() };
        let mcp = ClaudeCodeAdapter::mcp_config_path(&paths);
        let settings = ClaudeCodeAdapter::settings_path(&paths);
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = "{ \"mcpServers\": {} }\n";
        fs::write(&mcp, original).unwrap();
        fs::write(&settings, "{malformed").unwrap();

        for bridge in [Some("/test/ov-bridge"), None] {
            let error = ClaudeCodeAdapter::edit_configs(&paths, bridge).unwrap_err();
            assert!(error.contains(&settings.display().to_string()));
        }
        assert_eq!(fs::read_to_string(&mcp).unwrap(), original);
        assert_eq!(fs::read_to_string(&settings).unwrap(), "{malformed");
    }

    #[test]
    fn second_write_failure_rolls_back_first_write() {
        let tmp = tempdir().unwrap();
        let first_path = tmp.path().join("first.json");
        let original = "{ \"user\": true }\r\n";
        fs::write(&first_path, original).unwrap();
        let first = ConfigFile::read(first_path).unwrap();
        let parent = tmp.path().join("blocked");
        let second = ConfigFile::read(parent.join("second.json")).unwrap();
        fs::write(&parent, "not a directory").unwrap();

        let error = write_configs("test", &[(&first, Some("{}")), (&second, Some("{}"))]).unwrap_err();
        assert!(error.contains(&second.path.display().to_string()));
        assert_eq!(fs::read_to_string(first.path).unwrap(), original);
    }
}
