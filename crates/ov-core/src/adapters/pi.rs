use crate::adapters::{AgentAdapter, AgentStatus, ConnectOptions};
use crate::fs_util::atomic_write;
use crate::platform::{probe_cli_version, PlatformPaths};
use crate::policy::model::AgentId;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

pub struct PiAdapter;

impl PiAdapter {
    fn extension_dir(paths: &PlatformPaths) -> PathBuf {
        paths
            .home
            .join(".pi")
            .join("extensions")
            .join("openviking-bridge")
    }

    fn manifest_path(paths: &PlatformPaths) -> PathBuf {
        Self::extension_dir(paths).join("manifest.json")
    }

    fn is_connected(paths: &PlatformPaths) -> bool {
        Self::manifest_path(paths).is_file()
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

        AgentStatus {
            id: AgentId::Pi.as_str().to_string(),
            agent_id: AgentId::Pi,
            name: "pi Coding Agent".to_string(),
            integration_mode: "Native Extension Bridge".to_string(),
            installed,
            connected,
            binary_path: bin,
            config_path: Some(Self::extension_dir(paths)),
            version,
            warning: None,
            legacy_memory_files: self.scan_legacy_memories(paths),
        }
    }

    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let ext_dir = Self::extension_dir(paths);
        fs::create_dir_all(&ext_dir).map_err(|e| e.to_string())?;

        // Cross-platform copy + manifest strategy (avoids Windows symlink Developer Mode requirement).
        let manifest = json!({
            "name": "openviking-bridge",
            "version": env!("CARGO_PKG_VERSION"),
            "entry": "index.mjs",
            "bridge_binary": opts.bridge_binary.to_string_lossy(),
            "tool_prefix": "openviking_",
            "managed_by": "openviking-desktop"
        });
        let manifest_str = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
        atomic_write(&ext_dir.join("manifest.json"), manifest_str.as_bytes(), false)
            .map_err(|e| e.to_string())?;

        // Lightweight native pi extension that discovers tools dynamically from `ov-bridge mcp`.
        let bridge_json =
            serde_json::to_string(&opts.bridge_binary.to_string_lossy().to_string()).unwrap();
        let entry_js = format!(
            r#"// Managed by OpenViking Desktop — Native Extension Bridge for pi
import {{ spawn }} from "node:child_process";

const BRIDGE_BIN = {bridge_json};

function callBridgeMcp(method, params = {{}}) {{
  return new Promise((resolve, reject) => {{
    const proc = spawn(BRIDGE_BIN, ["mcp", "--agent", "pi"], {{ stdio: ["pipe", "pipe", "pipe"] }});
    let out = "";
    proc.stdout.on("data", (chunk) => {{ out += chunk.toString(); }});
    proc.on("error", reject);
    proc.on("close", () => {{
      const lines = out.trim().split("\n").filter(Boolean);
      for (let i = lines.length - 1; i >= 0; i--) {{
        try {{
          const msg = JSON.parse(lines[i]);
          if (msg.id === 1) return resolve(msg.result ?? msg.error);
        }} catch {{}}
      }}
      resolve(null);
    }});
    proc.stdin.write(JSON.stringify({{ jsonrpc: "2.0", id: 1, method, params }}) + "\n");
    proc.stdin.end();
  }});
}}

export default async function registerOpenViking(pi) {{
  const listed = await callBridgeMcp("tools/list");
  const tools = listed?.tools ?? [];
  for (const tool of tools) {{
    pi.registerTool({{
      name: `openviking_${{tool.name}}`,
      description: tool.description,
      parameters: tool.inputSchema,
      async execute(args) {{
        return callBridgeMcp("tools/call", {{ name: tool.name, arguments: args }});
      }},
    }});
  }}
}}
"#,
            bridge_json = bridge_json
        );

        atomic_write(&ext_dir.join("index.mjs"), entry_js.as_bytes(), false)
            .map_err(|e| e.to_string())?;

        Ok(self.detect(paths))
    }

    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String> {
        let ext_dir = Self::extension_dir(paths);
        if ext_dir.exists() {
            fs::remove_dir_all(&ext_dir).map_err(|e| e.to_string())?;
        }
        Ok(self.detect(paths))
    }

    fn scan_legacy_memories(&self, paths: &PlatformPaths) -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mem = paths.home.join(".pi").join("MEMORY.md");
        if mem.is_file() {
            files.push(mem);
        }
        files
    }
}
