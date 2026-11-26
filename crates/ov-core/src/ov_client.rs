use crate::fs_util::atomic_write;
use crate::platform::PlatformPaths;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
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
    http: Client,
}

impl OvClient {
    pub fn conf_path() -> Result<PathBuf, OvClientError> {
        let paths = PlatformPaths::detect().ok_or(OvClientError::HomeNotFound)?;
        Ok(paths.ovcli_conf())
    }

    pub fn profiles_path() -> Result<PathBuf, OvClientError> {
        let paths = PlatformPaths::detect().ok_or(OvClientError::HomeNotFound)?;
        Ok(paths.profiles_conf())
    }

    pub fn load_cli_config() -> Result<OvCliConfig, OvClientError> {
        if let Ok(env_path) = std::env::var("OPENVIKING_CLI_CONFIG_FILE") {
            let p = PathBuf::from(env_path);
            if p.exists() {
                let raw = std::fs::read_to_string(&p)
                    .map_err(|e| OvClientError::Config(format!("{}: {}", p.display(), e)))?;
                return serde_json::from_str(&raw)
                    .map_err(|e| OvClientError::Config(e.to_string()));
            }
        }
        let path = Self::conf_path()?;
        if !path.exists() {
            return Ok(OvCliConfig::default());
        }
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| OvClientError::Config(format!("{}: {}", path.display(), e)))?;
        serde_json::from_str(&raw).map_err(|e| OvClientError::Config(e.to_string()))
    }

    pub fn save_cli_config(cfg: &OvCliConfig) -> Result<PathBuf, OvClientError> {
        let path = Self::conf_path()?;
        let normalized = OvCliConfig {
            url: cfg.url.trim_end_matches('/').to_string(),
            api_key: cfg.api_key.trim().to_string(),
            account: cfg.account.clone(),
            user: cfg.user.clone(),
            timeout: if cfg.timeout <= 0.0 { 60.0 } else { cfg.timeout },
            output: if cfg.output.is_empty() {
                "json".to_string()
            } else {
                cfg.output.clone()
            },
        };
        let payload = serde_json::to_string_pretty(&normalized)
            .map_err(|e| OvClientError::Config(e.to_string()))?;
        atomic_write(&path, payload.as_bytes(), true)
            .map_err(|e| OvClientError::Config(e.to_string()))?;
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
        let path = Self::profiles_path()?;
        let payload = serde_json::to_string_pretty(store)
            .map_err(|e| OvClientError::Config(e.to_string()))?;
        atomic_write(&path, payload.as_bytes(), true)
            .map_err(|e| OvClientError::Config(e.to_string()))?;

        if let Some(active) = store.profiles.get(&store.active) {
            let cli_cfg = OvCliConfig {
                url: active.url.clone(),
                api_key: active.api_key.clone(),
                account: active.account.clone(),
                user: active.user.clone(),
                timeout: 60.0,
                output: "json".to_string(),
            };
            Self::save_cli_config(&cli_cfg)?;
        }
        Ok(path)
    }

    pub fn new(cfg: &OvCliConfig) -> Result<Self, OvClientError> {
        let mut headers = header::HeaderMap::new();
        if !cfg.api_key.trim().is_empty() {
            let key = cfg.api_key.trim();
            let bearer = header::HeaderValue::from_str(&format!("Bearer {}", key))
                .map_err(|e| OvClientError::Config(e.to_string()))?;
            headers.insert(header::AUTHORIZATION, bearer);
            let x_api_key = header::HeaderValue::from_str(key)
                .map_err(|e| OvClientError::Config(e.to_string()))?;
            headers.insert("X-API-Key", x_api_key);
        }
        if let Some(account) = &cfg.account {
            if !account.is_empty() {
                if let Ok(val) = header::HeaderValue::from_str(account) {
                    headers.insert("X-OpenViking-Account", val);
                }
            }
        }
        if let Some(user) = &cfg.user {
            if !user.is_empty() {
                if let Ok(val) = header::HeaderValue::from_str(user) {
                    headers.insert("X-OpenViking-User", val);
                }
            }
        }

        let timeout_secs = if cfg.timeout <= 0.0 { 60.0 } else { cfg.timeout };
        let http = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs_f64(timeout_secs))
            .build()?;

        Ok(Self {
            base_url: cfg.url.trim_end_matches('/').to_string(),
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
        self.get_json("/api/v1/fs/tree", &[("uri", uri)]).await
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

    pub async fn snapshot_restore(&self, snapshot_id: &str) -> Result<Value, OvClientError> {
        self.post_json(
            "/api/v1/snapshot/restore",
            &json!({ "snapshot_id": snapshot_id }),
        )
        .await
    }

    // ---- Compile (Wiki / Knowledge Compilation) APIs ----

    pub async fn trigger_compile(
        &self,
        compile_name: &str,
        source_uris: Vec<String>,
    ) -> Result<Value, OvClientError> {
        self.post_json(
            "/api/v1/compile",
            &json!({ "compile_name": compile_name, "source_uris": source_uris }),
        )
        .await
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
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| OvClientError::Config(format!("invalid json response: {}", e)))?;

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
