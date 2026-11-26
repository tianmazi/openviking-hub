pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod pi;
pub mod wsl;

use crate::platform::PlatformPaths;
use crate::policy::model::AgentId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub id: String,
    pub agent_id: AgentId,
    pub name: String,
    pub integration_mode: String,
    pub installed: bool,
    pub connected: bool,
    pub detected_surfaces: Vec<String>,
    pub binary_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub version: Option<String>,
    pub warning: Option<String>,
    pub legacy_memory_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectOptions {
    pub bridge_binary: PathBuf,
    pub write_project_rules: bool,
    pub project_dir: Option<PathBuf>,
}

pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;
    fn detect(&self, paths: &PlatformPaths) -> AgentStatus;
    fn connect(&self, paths: &PlatformPaths, opts: &ConnectOptions) -> Result<AgentStatus, String>;
    fn disconnect(&self, paths: &PlatformPaths) -> Result<AgentStatus, String>;
    fn scan_legacy_memories(&self, paths: &PlatformPaths) -> Vec<PathBuf>;
}

pub struct AdapterRegistry {
    adapters: Vec<Box<dyn AgentAdapter>>,
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: vec![
                Box::new(claude_code::ClaudeCodeAdapter),
                Box::new(codex::CodexAdapter),
                Box::new(pi::PiAdapter),
                Box::new(antigravity::AntigravityAdapter),
            ],
        }
    }

    pub fn detect_all(&self) -> Vec<AgentStatus> {
        let Some(paths) = PlatformPaths::detect() else {
            return Vec::new();
        };
        self.adapters.iter().map(|a| a.detect(&paths)).collect()
    }

    pub fn connect(&self, id: AgentId, opts: &ConnectOptions) -> Result<AgentStatus, String> {
        let paths = PlatformPaths::detect().ok_or("Cannot resolve home directory")?;
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| format!("Unknown agent adapter: {:?}", id))?;
        adapter.connect(&paths, opts)
    }

    pub fn disconnect(&self, id: AgentId) -> Result<AgentStatus, String> {
        let paths = PlatformPaths::detect().ok_or("Cannot resolve home directory")?;
        let adapter = self
            .adapters
            .iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| format!("Unknown agent adapter: {:?}", id))?;
        adapter.disconnect(&paths)
    }
}
