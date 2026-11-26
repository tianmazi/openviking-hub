use ov_core::adapters::wsl::inject_wsl_home;
use ov_core::adapters::{AdapterRegistry, AgentStatus, ConnectOptions};
use ov_core::dreaming::db::{DesktopDb, DreamingRun, RecallEvent};
use ov_core::dreaming::engine::{
    apply_accepted_changes as engine_apply, rollback_dreaming_run as engine_rollback,
    run_dreaming as engine_run, ApplyReport,
};
use ov_core::ov_client::{FindRequest, OvCliConfig, OvClient, ProfileStore, ServerStatus};
use ov_core::platform::{PlatformPaths, WslDistroInfo};
use ov_core::policy::model::{AgentId, PendingMemoryChange, PolicyConfig, ReviewStatus};
use ov_core::policy::store::PolicyStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayeredContent {
    pub uri: String,
    pub l0: String,
    pub l1: String,
    pub l2: String,
}

#[tauri::command]
pub async fn get_server_status() -> Result<ServerStatus, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    Ok(client.status().await)
}

#[tauri::command]
pub async fn test_connection(
    url: String,
    api_key: String,
    account: Option<String>,
    user: Option<String>,
) -> Result<ServerStatus, String> {
    let cfg = OvCliConfig {
        url,
        api_key,
        account,
        user,
        timeout: 15.0,
        output: "json".to_string(),
    };
    let client = OvClient::new(&cfg).map_err(|e| e.to_string())?;
    Ok(client.status().await)
}

#[tauri::command]
pub fn get_profiles() -> Result<ProfileStore, String> {
    OvClient::load_profiles().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_profiles(store: ProfileStore) -> Result<ProfileStore, String> {
    OvClient::save_profiles(&store).map_err(|e| e.to_string())?;
    Ok(store)
}

#[tauri::command]
pub fn detect_agents() -> Vec<AgentStatus> {
    AdapterRegistry::new().detect_all()
}

#[tauri::command]
pub fn detect_wsl() -> Vec<WslDistroInfo> {
    PlatformPaths::detect_wsl_distros()
}

#[tauri::command]
pub fn connect_wsl_agent(
    distro_name: String,
    home_unc: String,
    linux_bridge_path: Option<String>,
) -> Result<Vec<String>, String> {
    let bridge = linux_bridge_path
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "~/.openviking/bin/ov-bridge".to_string());
    inject_wsl_home(&distro_name, std::path::Path::new(&home_unc), &bridge)
}

#[tauri::command]
pub fn connect_agent(
    agent_id: AgentId,
    write_project_rules: bool,
    project_dir: Option<String>,
) -> Result<AgentStatus, String> {
    let paths = PlatformPaths::detect().ok_or("Cannot resolve home directory")?;
    let bridge_binary = paths.ensure_bridge_installed(None)?;
    let opts = ConnectOptions {
        bridge_binary,
        write_project_rules,
        project_dir: project_dir.map(PathBuf::from),
    };
    AdapterRegistry::new().connect(agent_id, &opts)
}

#[tauri::command]
pub fn disconnect_agent(agent_id: AgentId) -> Result<AgentStatus, String> {
    AdapterRegistry::new().disconnect(agent_id)
}

#[tauri::command]
pub async fn import_legacy_memory(local_path: String, target_uri: String) -> Result<Value, String> {
    let paths = PlatformPaths::detect().ok_or("Cannot resolve home directory")?;
    let resolved = paths.expand_tilde(std::path::Path::new(&local_path));
    let content = std::fs::read_to_string(&resolved)
        .map_err(|e| format!("Failed to read {}: {}", resolved.display(), e))?;
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    client
        .write_content(&target_uri, &content)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_policy() -> Result<PolicyConfig, String> {
    PolicyStore::load_or_init().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_policy(policy: PolicyConfig) -> Result<PolicyConfig, String> {
    PolicyStore::save(&policy).map_err(|e| e.to_string())?;
    Ok(policy)
}

#[tauri::command]
pub fn toggle_privacy_mode(capture_enabled: bool) -> Result<PolicyConfig, String> {
    let mut policy = PolicyStore::load_or_init().map_err(|e| e.to_string())?;
    policy.global.capture_enabled = capture_enabled;
    PolicyStore::save(&policy).map_err(|e| e.to_string())?;
    Ok(policy)
}

#[tauri::command]
pub async fn fs_ls(uri: String) -> Result<Value, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    client.ls(&uri).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fs_mkdir(uri: String) -> Result<Value, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    client.mkdir(&uri).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fs_delete(uri: String, recursive: bool) -> Result<Value, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    client
        .delete_fs(&uri, recursive)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_find(
    query: String,
    target_uri: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    client
        .find(&FindRequest {
            query,
            target_uri,
            limit,
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_uri_layers(uri: String) -> Result<LayeredContent, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    let l0 = extract_text(client.read_content(&uri, "l0").await.ok());
    let l1 = extract_text(client.read_content(&uri, "l1").await.ok());
    let l2 = extract_text(client.read_content(&uri, "l2").await.ok());
    Ok(LayeredContent { uri, l0, l1, l2 })
}

#[tauri::command]
pub async fn write_uri_content(uri: String, content: String) -> Result<Value, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    client
        .write_content(&uri, &content)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_recall_events(
    agent_filter: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<RecallEvent>, String> {
    let db = DesktopDb::open_default()?;
    db.list_events(agent_filter.as_deref(), limit.unwrap_or(100))
}

#[tauri::command]
pub async fn trigger_dreaming() -> Result<(DreamingRun, Vec<PendingMemoryChange>), String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    let db = DesktopDb::open_default()?;
    let policy = PolicyStore::load_or_init().map_err(|e| e.to_string())?;
    engine_run(&client, &db, &policy, "manual").await
}

#[tauri::command]
pub fn list_dreaming_runs(limit: Option<usize>) -> Result<Vec<DreamingRun>, String> {
    let db = DesktopDb::open_default()?;
    db.list_runs(limit.unwrap_or(20))
}

#[tauri::command]
pub fn list_pending_changes(
    status_filter: Option<String>,
) -> Result<Vec<PendingMemoryChange>, String> {
    let db = DesktopDb::open_default()?;
    db.list_pending_changes(status_filter.as_deref())
}

#[tauri::command]
pub fn update_change_decision(
    id: String,
    status: ReviewStatus,
    edited_after_content: Option<String>,
) -> Result<(), String> {
    let db = DesktopDb::open_default()?;
    db.update_change_decision(&id, status, edited_after_content.as_deref())
}

#[tauri::command]
pub async fn apply_accepted_changes() -> Result<ApplyReport, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    let db = DesktopDb::open_default()?;
    let policy = PolicyStore::load_or_init().map_err(|e| e.to_string())?;
    engine_apply(&client, &db, &policy).await
}

#[tauri::command]
pub async fn rollback_dreaming_run(run_id: String) -> Result<Value, String> {
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    let db = DesktopDb::open_default()?;
    engine_rollback(&client, &db, &run_id).await
}

#[tauri::command]
pub fn pin_memory_uri(uri: String) -> Result<(), String> {
    let db = DesktopDb::open_default()?;
    db.pin_uri(&uri)
}

fn extract_text(val: Option<Value>) -> String {
    let Some(v) = val else {
        return String::new();
    };
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    v.get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| serde_json::to_string_pretty(&v).unwrap_or_default())
}
