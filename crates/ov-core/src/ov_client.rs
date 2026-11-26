use crate::fs_util::{atomic_write, sha256_hex};
use crate::platform::PlatformPaths;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum OvClientError {
    #[error("cannot resolve home directory")]
    HomeNotFound,
    #[error("config error: {0}")]
    Config(String),
    #[error("http transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("server returned status {status} ({code}): {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
}

/// Matches the official `~/.openviking/ovcli.conf` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvCliConfig {
    pub url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout: f64,
    #[serde(default = "default_output_format")]
    pub output: String,
}

fn default_timeout_secs() -> f64 {
    60.0
}

fn default_output_format() -> String {
    "json".to_string()
}

impl Default for OvCliConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:1933".to_string(),
            api_key: String::new(),
            account: None,
            user: None,
            timeout: 60.0,
            output: "json".to_string(),
        }
    }
}

/// Multi-profile registry stored in `~/.openviking/profiles.json`.
/// Switching the active profile atomically syncs `~/.openviking/ovcli.conf`
/// so `ov-bridge` and all local agents immediately pick up the new target server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileStore {
    pub active: String,
    pub profiles: HashMap<String, OvProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvProfile {
    pub name: String,
    pub label: String,
    pub url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

impl Default for ProfileStore {
    fn default() -> Self {
        let mut profiles = HashMap::new();
        profiles.insert(
            "default".to_string(),
            OvProfile {
                name: "default".to_string(),
                label: "Local / Default OpenViking".to_string(),
                url: "http://127.0.0.1:1933".to_string(),
                api_key: String::new(),
                account: None,
                user: None,
            },
        );
        Self {
            active: "default".to_string(),
            profiles,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub reachable: bool,
    pub latency_ms: u128,
    pub url: String,
    pub system_status: Option<Value>,
    pub observer: Option<Value>,
    pub error: Option<String>,
}

/// Official OpenViking REST API client aligned with `https://docs.openviking.ai/en/api/01-overview`.
#[derive(Clone)]
pub struct OvClient {
    base_url: String,
    profile_key: String,
    http: Client,
}

static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

fn read_optional_config(path: &Path) -> Result<Option<Vec<u8>>, OvClientError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(OvClientError::Config(format!("{}: {e}", path.display()))),
    }
}

fn write_cli_config(
    path: &Path,
    cfg: &OvCliConfig,
    existing: Option<&[u8]>,
) -> Result<(), OvClientError> {
    OvClient::new(cfg)?;
    let mut fields = existing
        .and_then(|bytes| serde_json::from_slice::<serde_json::Map<String, Value>>(bytes).ok())
        .unwrap_or_default();
    fields.remove("account");
    fields.remove("user");
    fields.extend([
        (
            "url".to_string(),
            json!(cfg.url.trim().trim_end_matches('/')),
        ),
        ("api_key".to_string(), json!(cfg.api_key.trim())),
        ("timeout".to_string(), json!(timeout_secs(cfg.timeout))),
        (
            "output".to_string(),
            json!(if cfg.output.is_empty() { "json" } else { cfg.output.as_str() }),
        ),
    ]);
    for (field, value) in [("account", &cfg.account), ("user", &cfg.user)] {
        if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            fields.insert(field.to_string(), json!(value));
        }
    }
    let payload =
        serde_json::to_vec_pretty(&fields).map_err(|e| OvClientError::Config(e.to_string()))?;
    atomic_write(path, &payload, true).map_err(|e| OvClientError::Config(e.to_string()))
}

fn timeout_secs(timeout: f64) -> f64 {
    if timeout > 0.0 && Duration::try_from_secs_f64(timeout).is_ok() {
        timeout
    } else {
        default_timeout_secs()
    }
}

pub(crate) fn snapshot_commit_id(value: &Value) -> Option<&str> {
    ["commit_oid", "snapshot_id", "id", "commit_id"]
        .into_iter()
        .filter_map(|field| value.get(field).and_then(Value::as_str))
        .map(str::trim)
        .find(|id| !id.is_empty())
}

impl OvClient {
    pub fn conf_path() -> Result<PathBuf, OvClientError> {
        if let Some(path) = std::env::var_os("OPENVIKING_CLI_CONFIG_FILE") {
            return Ok(PathBuf::from(path));
        }
        let paths = PlatformPaths::detect().ok_or(OvClientError::HomeNotFound)?;
        Ok(paths.ovcli_conf())
    }

    pub fn profiles_path() -> Result<PathBuf, OvClientError> {
        let paths = PlatformPaths::detect().ok_or(OvClientError::HomeNotFound)?;
        Ok(paths.profiles_conf())
    }

    pub fn load_cli_config() -> Result<OvCliConfig, OvClientError> {
        let path = Self::conf_path()?;
        let Some(raw) = read_optional_config(&path)? else {
            return Ok(OvCliConfig::default());
        };
        serde_json::from_slice(&raw)
            .map_err(|e| OvClientError::Config(format!("{}: {e}", path.display())))
    }

    pub fn save_cli_config(cfg: &OvCliConfig) -> Result<PathBuf, OvClientError> {
        let _guard = CONFIG_WRITE_LOCK
            .lock()
            .map_err(|e| OvClientError::Config(e.to_string()))?;
        let path = Self::conf_path()?;
        let existing = read_optional_config(&path)?;
        write_cli_config(&path, cfg, existing.as_deref())?;
        Ok(path)
    }

    pub fn load_profiles() -> Result<ProfileStore, OvClientError> {
        let path = Self::profiles_path()?;
        if !path.exists() {
            let mut store = ProfileStore::default();
            if let Ok(existing) = Self::load_cli_config() {
                if let Some(def) = store.profiles.get_mut("default") {
                    def.url = existing.url;
                    def.api_key = existing.api_key;
                    def.account = existing.account;
                    def.user = existing.user;
                }
            }
            return Ok(store);
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| OvClientError::Config(e.to_string()))?;
        serde_json::from_str(&raw).map_err(|e| OvClientError::Config(e.to_string()))
    }

    pub fn save_profiles(store: &ProfileStore) -> Result<PathBuf, OvClientError> {
        let _guard = CONFIG_WRITE_LOCK
            .lock()
            .map_err(|e| OvClientError::Config(e.to_string()))?;
        let path = Self::profiles_path()?;
        let cli_path = Self::conf_path()?;
        if cli_path == path
            || matches!(
                (std::fs::canonicalize(&cli_path), std::fs::canonicalize(&path)),
                (Ok(cli), Ok(profiles)) if cli == profiles
            )
        {
            return Err(OvClientError::Config(
                "profiles and CLI config share a path".into(),
            ));
        }
        let active = store.profiles.get(&store.active).ok_or_else(|| {
            OvClientError::Config(format!("missing active profile: {}", store.active))
        })?;
        let payload =
            serde_json::to_vec_pretty(store).map_err(|e| OvClientError::Config(e.to_string()))?;
        let existing = read_optional_config(&cli_path)?;
        let mut cli_cfg = existing
            .as_deref()
            .and_then(|raw| serde_json::from_slice::<OvCliConfig>(raw).ok())
            .unwrap_or_default();
        cli_cfg.url.clone_from(&active.url);
        cli_cfg.api_key.clone_from(&active.api_key);
        cli_cfg.account.clone_from(&active.account);
        cli_cfg.user.clone_from(&active.user);
        write_cli_config(&cli_path, &cli_cfg, existing.as_deref())?;

        if let Err(e) = atomic_write(&path, &payload, true) {
            let restored = match existing {
                Some(raw) => atomic_write(&cli_path, &raw, true),
                None => std::fs::remove_file(&cli_path).map_err(Into::into),
            };
            return Err(OvClientError::Config(match restored {
                Ok(()) => e.to_string(),
                Err(restore_err) => format!("{e}; failed to restore CLI config: {restore_err}"),
            }));
        }
        Ok(path)
    }

    pub fn new(cfg: &OvCliConfig) -> Result<Self, OvClientError> {
        let url =
            reqwest::Url::parse(cfg.url.trim()).map_err(|e| OvClientError::Config(e.to_string()))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(OvClientError::Config(
                "expected an HTTP(S) base URL without a query or fragment".into(),
            ));
        }
        let base_url = url.as_str().trim_end_matches('/').to_string();
        let account = cfg.account.as_deref().unwrap_or_default().trim();
        let user = cfg.user.as_deref().unwrap_or_default().trim();
        let key = cfg.api_key.trim();
        let fingerprint = if key.is_empty() {
            String::new()
        } else {
            sha256_hex(key)[..12].to_owned()
        };
        let profile_key = sha256_hex(&json!([base_url, account, user, fingerprint]).to_string());
        let mut headers = header::HeaderMap::new();
        if !key.is_empty() {
            let bearer = header::HeaderValue::from_str(&format!("Bearer {}", key))
                .map_err(|e| OvClientError::Config(e.to_string()))?;
            headers.insert(header::AUTHORIZATION, bearer);
            let x_api_key = header::HeaderValue::from_str(key)
                .map_err(|e| OvClientError::Config(e.to_string()))?;
            headers.insert("X-API-Key", x_api_key);
        }
        for (name, identity) in [("X-OpenViking-Account", account), ("X-OpenViking-User", user)] {
            if !identity.is_empty() {
                let value = header::HeaderValue::from_str(identity)
                    .map_err(|e| OvClientError::Config(e.to_string()))?;
                headers.insert(name, value);
            }
        }

        let http = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs_f64(timeout_secs(cfg.timeout)))
            .build()?;

        Ok(Self {
            base_url,
            profile_key,
            http,
        })
    }

    pub fn from_local_config() -> Result<Self, OvClientError> {
        let cfg = Self::load_cli_config()?;
        Self::new(&cfg)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn profile_key(&self) -> String {
        self.profile_key.clone()
    }

    /// Check `/health`, `/api/v1/system/status`, and `/api/v1/observer/system`.
    pub async fn status(&self) -> ServerStatus {
        let start = std::time::Instant::now();
        let health_url = format!("{}/health", self.base_url);
        match self.http.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let sys = self.get_json("/api/v1/system/status", &[]).await.ok();
                let obs = self.get_json("/api/v1/observer/system", &[]).await.ok();
                ServerStatus {
                    reachable: true,
                    latency_ms: start.elapsed().as_millis(),
                    url: self.base_url.clone(),
                    system_status: sys,
                    observer: obs,
                    error: None,
                }
            }
            Ok(resp) => ServerStatus {
                reachable: false,
                latency_ms: start.elapsed().as_millis(),
                url: self.base_url.clone(),
                system_status: None,
                observer: None,
                error: Some(format!("HTTP {}", resp.status().as_u16())),
            },
            Err(err) => ServerStatus {
                reachable: false,
                latency_ms: start.elapsed().as_millis(),
                url: self.base_url.clone(),
                system_status: None,
                observer: None,
                error: Some(err.to_string()),
            },
        }
    }

    // ---- Filesystem & Content APIs ----

    pub async fn ls(&self, uri: &str) -> Result<Value, OvClientError> {
        self.get_json("/api/v1/fs/ls", &[("uri", uri)]).await
    }

    pub async fn tree(&self, uri: &str) -> Result<Value, OvClientError> {
        self.get_json("/api/v1/fs/tree", &[("uri", uri), ("output", "original")])
            .await
    }

    pub async fn stat(&self, uri: &str) -> Result<Value, OvClientError> {
        self.get_json("/api/v1/fs/stat", &[("uri", uri)]).await
    }

    pub async fn mkdir(&self, uri: &str) -> Result<Value, OvClientError> {
        self.post_json("/api/v1/fs/mkdir", &json!({ "uri": uri }))
            .await
    }

    pub async fn mv(&self, from_uri: &str, to_uri: &str) -> Result<Value, OvClientError> {
        self.post_json(
            "/api/v1/fs/mv",
            &json!({ "from_uri": from_uri, "to_uri": to_uri }),
        )
        .await
    }

    pub async fn delete_fs(&self, uri: &str, recursive: bool) -> Result<Value, OvClientError> {
        let url = format!("{}/api/v1/fs", self.base_url);
        let resp = self
            .http
            .delete(&url)
            .query(&[
                ("uri", uri),
                ("recursive", if recursive { "true" } else { "false" }),
            ])
            .send()
            .await?;
        self.parse_envelope(resp).await
    }

    pub async fn read_content(&self, uri: &str, layer: &str) -> Result<Value, OvClientError> {
        let endpoint = match layer {
            "l0" | "abstract" => "/api/v1/content/abstract",
            "l1" | "overview" => "/api/v1/content/overview",
            _ => "/api/v1/content/read",
        };
        self.get_json(endpoint, &[("uri", uri)]).await
    }

    pub async fn write_content(&self, uri: &str, content: &str) -> Result<Value, OvClientError> {
        self.post_json(
            "/api/v1/content/write",
            &json!({ "uri": uri, "content": content }),
        )
        .await
    }

    // ---- Retrieval APIs ----

    pub async fn find(&self, req: &FindRequest) -> Result<Value, OvClientError> {
        self.post_json("/api/v1/search/find", req).await
    }

    pub async fn search_context(&self, req: &ContextSearchRequest) -> Result<Value, OvClientError> {
        self.post_json("/api/v1/search/search", req).await
    }

    pub async fn grep(&self, pattern: &str, target_uri: Option<&str>) -> Result<Value, OvClientError> {
        self.post_json(
            "/api/v1/search/grep",
            &json!({ "pattern": pattern, "target_uri": target_uri }),
        )
        .await
    }

    // ---- Session APIs ----

    pub async fn create_session(&self, metadata: Value) -> Result<Value, OvClientError> {
        self.post_json("/api/v1/sessions", &metadata).await
    }

    pub async fn add_session_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<Value, OvClientError> {
        let path = format!("/api/v1/sessions/{}/messages", session_id);
        self.post_json(&path, &json!({ "role": role, "content": content }))
            .await
    }

    pub async fn commit_session(&self, session_id: &str) -> Result<Value, OvClientError> {
        let path = format!("/api/v1/sessions/{}/commit", session_id);
        self.post_json(&path, &json!({})).await
    }

    // ---- Snapshots (Multi-Version Management) APIs ----

    pub async fn snapshot_commit(
        &self,
        message: &str,
        paths: Option<Vec<String>>,
    ) -> Result<Value, OvClientError> {
        self.post_json(
            "/api/v1/snapshot/commit",
            &json!({ "message": message, "paths": paths }),
        )
        .await
    }

    pub async fn snapshot_log(&self, limit: usize) -> Result<Value, OvClientError> {
        let limit_str = limit.to_string();
        self.get_json("/api/v1/snapshot/log", &[("limit", &limit_str)])
            .await
    }

    pub async fn snapshot_restore(&self, source_commit: &str) -> Result<Value, OvClientError> {
        self.snapshot_restore_scoped(source_commit, None).await
    }

    pub async fn snapshot_restore_scoped(
        &self,
        source_commit: &str,
        project_dir: Option<&str>,
    ) -> Result<Value, OvClientError> {
        let mut body = json!({ "source_commit": source_commit });
        if let Some(dir) = project_dir {
            body["project_dir"] = json!(dir);
        }
        self.post_json("/api/v1/snapshot/restore", &body).await
    }

    // ---- Compile (Wiki / Knowledge Compilation) APIs ----

    pub async fn trigger_compile(
        &self,
        to: &str,
        source_uris: Vec<String>,
    ) -> Result<Value, OvClientError> {
        let mut body = json!({ "to": to, "from": source_uris, "skill": "wiki" });
        match self.post_json("/api/v1/compile", &body).await {
            Err(OvClientError::Api { status: 400, code, .. }) if code == "INVALID_URI" => {
                // The v0.5.2 server requires an installed Skill URI.
                body["skill"] = json!("viking://agent/skills/llm-wiki");
                self.post_json("/api/v1/compile", &body).await
            }
            result => result,
        }
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Value, OvClientError> {
        let path = format!("/api/v1/tasks/{}", task_id);
        self.get_json(&path, &[]).await
    }

    // ---- Internal HTTP helpers ----

    async fn get_json(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, OvClientError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.get(&url).query(query).send().await?;
        self.parse_envelope(resp).await
    }

    async fn post_json<T: Serialize>(&self, path: &str, body: &T) -> Result<Value, OvClientError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.http.post(&url).json(body).send().await?;
        self.parse_envelope(resp).await
    }

    async fn parse_envelope(&self, resp: reqwest::Response) -> Result<Value, OvClientError> {
        let status = resp.status();
        let text = resp.text().await?;
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(_) if !status.is_success() => Value::Null,
            Err(e) => return Err(OvClientError::Config(format!("invalid json response: {e}"))),
        };

        if !status.is_success() || parsed.get("status").and_then(|s| s.as_str()) == Some("error") {
            let err_obj = parsed.get("error");
            let code = err_obj
                .and_then(|e| e.get("code"))
                .and_then(|c| c.as_str())
                .unwrap_or("HTTP_ERROR")
                .to_string();
            let message = err_obj
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or(&text)
                .to_string();
            return Err(OvClientError::Api {
                status: status.as_u16(),
                code,
                message,
            });
        }

        if let Some(result) = parsed.get("result") {
            Ok(result.clone())
        } else {
            Ok(parsed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_config_write_preserves_unmanaged_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ovcli.conf");
        let existing = br#"{
            "account": "old",
            "user": "old",
            "plugin": {"name": "example"},
            "extra_headers": {"custom-header": "value"},
            "actor_peer_id": "peer"
        }"#;
        let cfg = OvCliConfig { user: Some("new".into()), ..Default::default() };
        write_cli_config(&path, &cfg, Some(existing)).unwrap();

        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["plugin"]["name"], json!("example"));
        assert_eq!(saved["extra_headers"]["custom-header"], json!("value"));
        assert_eq!(saved["actor_peer_id"], json!("peer"));
        assert_eq!(saved["user"], json!("new"));
        assert!(saved.get("account").is_none());
    }

    #[test]
    fn profile_key_tracks_normalized_url_and_identity() {
        let mut cfg = OvCliConfig {
            url: " HTTPS://Example.com:443/api/// ".into(),
            account: Some("team".into()),
            user: Some("alice".into()),
            ..Default::default()
        };
        let original = OvClient::new(&cfg).unwrap().profile_key();
        cfg.url = "https://example.com/api".into();
        cfg.user = Some(" alice ".into());
        assert_eq!(OvClient::new(&cfg).unwrap().profile_key(), original);
        cfg.account = Some("other-team".into());
        assert_ne!(OvClient::new(&cfg).unwrap().profile_key(), original);
    }

    #[test]
    fn user_api_keys_isolate_profiles_without_identity_headers() {
        let mut cfg = OvCliConfig { api_key: " first-key ".into(), ..Default::default() };
        let original = OvClient::new(&cfg).unwrap().profile_key();
        cfg.api_key = "first-key".into();
        assert_eq!(OvClient::new(&cfg).unwrap().profile_key(), original);
        cfg.api_key = "second-key".into();
        assert_ne!(OvClient::new(&cfg).unwrap().profile_key(), original);
    }

    #[test]
    fn invalid_cli_config_leaves_existing_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ovcli.conf");
        let mut cfg = OvCliConfig::default();
        write_cli_config(&path, &cfg, None).unwrap();
        let existing = std::fs::read(&path).unwrap();

        cfg.url = "invalid-url".into();
        assert!(write_cli_config(&path, &cfg, Some(&existing)).is_err());
        cfg.url = OvCliConfig::default().url;
        cfg.api_key = "invalid\nkey".into();
        assert!(write_cli_config(&path, &cfg, Some(&existing)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), existing);
    }

    #[test]
    fn snapshot_id_prefers_commit_oid_and_skips_invalid_ids() {
        let modern = json!({"commit_oid": "new", "snapshot_id": "legacy"});
        let legacy = json!({"commit_oid": "", "snapshot_id": null, "id": "old"});
        assert_eq!(snapshot_commit_id(&modern), Some("new"));
        assert_eq!(snapshot_commit_id(&legacy), Some("old"));
        assert_eq!(snapshot_commit_id(&json!({"commit_oid": " "})), None);
    }
}
