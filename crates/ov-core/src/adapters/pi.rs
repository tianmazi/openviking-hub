use super::claude_code::{edit_mcp_server, json_path_exists, lock_configs, write_configs, ConfigFile};
use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::json;
use std::path::PathBuf;

pub struct PiAdapter;

impl PiAdapter {
    fn mcp_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".pi/agent/mcp.json")
    }

    fn extension_path(paths: &PlatformPaths) -> PathBuf {
        paths.home.join(".pi/agent/extensions/openviking-bridge.js")
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        json_path_exists(Self::mcp_path(paths), &["mcpServers", "openviking"])
            && Self::extension_path(paths).is_file()
    }
}

impl AgentAdapter for PiAdapter {
    fn id(&self) -> AgentId {
        AgentId::Pi
    }

    fn detect(&self, paths: &PlatformPaths) -> AgentStatus {
        let bin = paths.resolve_binary("pi");
        let dir_exists = paths.home.join(".pi").exists();
        let installed = bin.is_some() || dir_exists;
        let version = bin.as_deref().and_then(probe_cli_version);
        let connected = Self::is_connected(paths);

        let mut detected_surfaces = Vec::new();
        if installed {
            detected_surfaces.push("CLI".to_string());
        }

        AgentStatus {
            id: AgentId::Pi.as_str().to_string(),
            agent_id: AgentId::Pi,
            name: "pi Coding Agent".to_string(),
            integration_mode: "MCP + Lifecycle Extension".to_string(),
            installed,
            connected,
            detected_surfaces,
            binary_path: bin,
            config_path: Some(Self::mcp_path(paths)),
            version,
            warning: None,
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let lock = lock_configs()?;
        let mcp = ConfigFile::read(Self::mcp_path(paths))?;
        let extension = ConfigFile::read(Self::extension_path(paths))?;
        let bridge = opts.bridge_binary.to_string_lossy();
        let mcp_content = edit_mcp_server(&mcp, Some(json!({
            "command": bridge,
            "args": ["mcp", "--agent", "pi"],
            "exposure": "direct",
            "description": "OpenViking shared memory recall and capture tools"
        })))?;
        let bridge_json = serde_json::to_string(&bridge).map_err(|e| e.to_string())?;
        let extension_content = format!("const BRIDGE_BIN = {bridge_json};\n{LIFECYCLE_EXTENSION}");
        write_configs("pi", &[
            (&mcp, mcp_content.as_deref()),
            (&extension, Some(&extension_content)),
        ])?;
        drop(lock);
        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        let lock = lock_configs()?;
        let mcp = ConfigFile::read(Self::mcp_path(paths))?;
        let extension = ConfigFile::read(Self::extension_path(paths))?;
        let mcp_content = edit_mcp_server(&mcp, None)?;
        let legacy_dir = paths.home.join(".pi/extensions/openviking-bridge");
        let manifest = ConfigFile::read(legacy_dir.join("manifest.json"))?;
        let legacy_extension;
        let mut edits = vec![(&mcp, mcp_content.as_deref()), (&extension, None)];
        if manifest.json()?.get("managed_by").and_then(serde_json::Value::as_str)
            == Some("openviking-desktop")
        {
            legacy_extension = ConfigFile::read(legacy_dir.join("index.mjs"))?;
            edits.extend([(&legacy_extension, None), (&manifest, None)]);
        }
        write_configs("pi", &edits)?;
        drop(lock);
        Ok(self.detect(paths))
    }

    fn scan_legacy_memories(&self, paths: &PlatformPaths) -> Vec<PathBuf> {
        [".pi/agent/MEMORY.md", ".pi/MEMORY.md"].iter()
            .map(|path| paths.home.join(path))
            .filter(|path| path.is_file())
            .collect()
    }
}

const LIFECYCLE_EXTENSION: &str = r#"import { execFileSync } from "node:child_process";

function runHook(event, payload) {
  try {
    return execFileSync(BRIDGE_BIN, ["hook", event, "--agent", "pi"], {
      input: JSON.stringify(payload),
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
      timeout: 30000,
    }).trim();
  } catch {
    return "";
  }
}

export default function (pi) {
  const sessionPayload = (ctx, event) => {
    const manager = ctx.sessionManager;
    const payload = { cwd: ctx.cwd, session_id: manager.getSessionId() };
    const branch = manager.getBranch?.();
    if (Array.isArray(branch)) {
      // Preserve entry IDs so repeated scans and existing JSONL watermarks still match.
      return { ...payload, messages: branch.filter((entry) => entry.type === "message") };
    }
    if (Array.isArray(event?.messages)) {
      const messages = event.messages.map((message) => {
        if (message.id || message.uuid || message.timestamp == null) return message;
        return {
          ...message,
          id: `pi:${message.role}:${message.timestamp}:${JSON.stringify(message.content)}`,
        };
      });
      return { ...payload, messages };
    }
    return { ...payload, transcript_path: manager.getSessionFile() };
  };

  let startup = "";
  pi.on("session_start", async (_event, ctx) => {
    startup = runHook("session-start", sessionPayload(ctx));
  });

  pi.on("before_agent_start", async (event, ctx) => {
    const recall = runHook("user-prompt-submit", { ...sessionPayload(ctx), prompt: event.prompt });
    const context = [startup, recall].filter(Boolean).join("\n\n");
    if (context && typeof event.systemPrompt === "string") {
      return { systemPrompt: event.systemPrompt + "\n\n" + context };
    }
  });

  pi.on("agent_end", async (event, ctx) => {
    runHook("stop", sessionPayload(ctx, event));
  });
  pi.on("session_before_compact", async (_event, ctx) => {
    runHook("pre-compact", sessionPayload(ctx));
  });
  pi.on("session_shutdown", async (_event, ctx) => {
    runHook("session-end", sessionPayload(ctx));
  });
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_preserves_mcp_servers_and_unmanaged_files() {
        let tmp = tempdir().unwrap();
        let paths = PlatformPaths { home: tmp.path().to_path_buf() };
        let mcp = PiAdapter::mcp_path(&paths);
        let extension = PiAdapter::extension_path(&paths);
        fs::create_dir_all(mcp.parent().unwrap()).unwrap();
        let user = json!({ "mcpServers": { "magpie": { "command": "magpie", "exposure": "direct" } } });
        fs::write(&mcp, user.to_string()).unwrap();
        let legacy = tmp.path().join(".pi/extensions/openviking-bridge");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("manifest.json"), r#"{"managed_by":"openviking-desktop"}"#).unwrap();
        fs::write(legacy.join("index.mjs"), "// old extension").unwrap();
        fs::write(legacy.join("user.txt"), "preserve").unwrap();
        fs::write(tmp.path().join(".pi/MEMORY.md"), "").unwrap();
        fs::write(tmp.path().join(".pi/agent/MEMORY.md"), "").unwrap();

        let opts = ConnectOptions {
            bridge_binary: PathBuf::from("/test/ov-bridge"),
            write_project_rules: false,
            project_dir: None,
        };
        assert!(PiAdapter.connect(&paths, &opts).unwrap().connected);
        let read_mcp = || serde_json::from_str::<Value>(&fs::read_to_string(&mcp).unwrap()).unwrap();
        let connected = read_mcp();
        assert_eq!(connected["mcpServers"]["magpie"], user["mcpServers"]["magpie"]);
        assert_eq!(connected["mcpServers"]["openviking"], json!({
            "command": "/test/ov-bridge",
            "args": ["mcp", "--agent", "pi"],
            "exposure": "direct",
            "description": "OpenViking shared memory recall and capture tools"
        }));
        let js = fs::read_to_string(&extension).unwrap();
        assert!(js.contains("before_agent_start") && js.contains("agent_end"));
        assert!(!js.contains("registerTool"));
        assert_eq!(PiAdapter.scan_legacy_memories(&paths).len(), 2);

        assert!(!PiAdapter.disconnect(&paths).unwrap().connected);
        assert_eq!(read_mcp(), user);
        assert!(!extension.exists());
        assert!(!legacy.join("manifest.json").exists());
        assert!(!legacy.join("index.mjs").exists());
        assert!(legacy.join("user.txt").is_file());

        fs::write(legacy.join("manifest.json"), r#"{"managed_by":"other"}"#).unwrap();
        fs::write(legacy.join("index.mjs"), "// user extension").unwrap();
        PiAdapter.disconnect(&paths).unwrap();
        assert!(legacy.join("manifest.json").is_file());
        assert!(legacy.join("index.mjs").is_file());
    }

    #[test]
    fn corrupt_mcp_config_preserves_existing_extension() {
        let tmp = tempdir().unwrap();
        let paths = PlatformPaths { home: tmp.path().to_path_buf() };
        let mcp = PiAdapter::mcp_path(&paths);
        let extension = PiAdapter::extension_path(&paths);
        fs::create_dir_all(extension.parent().unwrap()).unwrap();
        fs::write(&mcp, "{invalid").unwrap();
        fs::write(&extension, "// keep").unwrap();
        let error = PiAdapter.disconnect(&paths).unwrap_err();
        assert!(error.contains(&mcp.display().to_string()));
        assert_eq!(fs::read_to_string(extension).unwrap(), "// keep");
    }
}
