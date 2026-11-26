use crate::mcp::truncate_to_token_budget;
use chrono::Utc;
use ov_core::dreaming::db::{DesktopDb, RecallEvent};
use ov_core::ov_client::{ContextSearchRequest, OvClient};
use ov_core::policy::model::AgentId;
use ov_core::policy::router::resolve_route;
use ov_core::policy::store::PolicyStore;
use serde_json::{json, Value};
use std::io::{self, Read};
use std::time::Instant;
use uuid::Uuid;

/// Execute cross-platform lifecycle Hooks (`session-start`, `user-prompt-submit`, `stop`)
/// without requiring Bash, PowerShell, Node.js, or Python on the host machine.
pub async fn run_hook(event: &str, agent: AgentId) -> anyhow::Result<()> {
    let start = Instant::now();
    let mut stdin_buf = String::new();
    let _ = io::stdin().read_to_string(&mut stdin_buf);
    let payload: Value = serde_json::from_str(&stdin_buf).unwrap_or_else(|_| json!({}));

    let policy = PolicyStore::load_or_init()?;
    let budget = policy.budget.resolve_for(agent);
    let client = match OvClient::from_local_config() {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    let db = DesktopDb::open_default().ok();

    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let route = resolve_route(&policy, &cwd);

    match event {
        "session-start" => {
            if let Ok(res) = client
                .read_content(&route.target_resource_uri, budget.default_recall_layer.as_str())
                .await
            {
                let mut text = res
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| serde_json::to_string(&res).unwrap_or_default());
                // Enforce startup line & byte caps (e.g. 9500 bytes for Claude Code SessionStart)
                text = enforce_startup_caps(&text, budget.startup_max_lines, budget.startup_max_bytes);
                if !text.trim().is_empty() {
                    println!("{}", text);
                }
                if let Some(db) = db {
                    let _ = db.record_event(&RecallEvent {
                        id: Uuid::new_v4().to_string(),
                        agent_id: agent.as_str().to_string(),
                        event_type: "hook_start".to_string(),
                        query: None,
                        target_uri: Some(route.target_resource_uri),
                        layer: Some(budget.default_recall_layer.as_str().to_string()),
                        tokens_used: (text.len() / 4) as i64,
                        latency_ms: start.elapsed().as_millis() as i64,
                        blocked_by_privacy: false,
                        created_at: Utc::now().timestamp(),
                    });
                }
            }
        }
        "user-prompt-submit" => {
            let prompt = payload
                .get("prompt")
                .or_else(|| payload.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if prompt.trim().is_empty() {
                return Ok(());
            }
            if let Ok(res) = client
                .search_context(&ContextSearchRequest {
                    query: prompt.to_string(),
                    target_uri: Some(route.target_resource_uri.clone()),
                    mode: Some("context".to_string()),
                    limit: Some(5),
                })
                .await
            {
                let text = truncate_to_token_budget(
                    &serde_json::to_string_pretty(&res).unwrap_or_default(),
                    budget.max_recall_tokens,
                );
                if !text.trim().is_empty() {
                    println!("{}", text);
                }
                if let Some(db) = db {
                    let _ = db.record_event(&RecallEvent {
                        id: Uuid::new_v4().to_string(),
                        agent_id: agent.as_str().to_string(),
                        event_type: "recall".to_string(),
                        query: Some(prompt.chars().take(120).collect()),
                        target_uri: Some(route.target_resource_uri),
                        layer: Some(budget.default_recall_layer.as_str().to_string()),
                        tokens_used: (text.len() / 4) as i64,
                        latency_ms: start.elapsed().as_millis() as i64,
                        blocked_by_privacy: false,
                        created_at: Utc::now().timestamp(),
                    });
                }
            }
        }
        "stop" => {
            if !policy.global.capture_enabled {
                if let Some(db) = db {
                    let _ = db.record_event(&RecallEvent {
                        id: Uuid::new_v4().to_string(),
                        agent_id: agent.as_str().to_string(),
                        event_type: "hook_stop".to_string(),
                        query: None,
                        target_uri: Some(route.target_memory_uri),
                        layer: None,
                        tokens_used: 0,
                        latency_ms: start.elapsed().as_millis() as i64,
                        blocked_by_privacy: true,
                        created_at: Utc::now().timestamp(),
                    });
                }
                return Ok(());
            }

            let summary = payload
                .get("transcript_summary")
                .or_else(|| payload.get("last_assistant_message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !summary.trim().is_empty() {
                if let Ok(sess) = client
                    .create_session(json!({
                        "agent": agent.as_str(),
                        "resource_uri": route.target_resource_uri
                    }))
                    .await
                {
                    if let Some(sid) = sess
                        .get("session_id")
                        .or_else(|| sess.get("id"))
                        .and_then(|v| v.as_str())
                    {
                        let _ = client.add_session_message(sid, "assistant", summary).await;
                        let _ = client.commit_session(sid).await;
                    }
                }
            }
            if let Some(db) = db {
                let _ = db.record_event(&RecallEvent {
                    id: Uuid::new_v4().to_string(),
                    agent_id: agent.as_str().to_string(),
                    event_type: "hook_stop".to_string(),
                    query: None,
                    target_uri: Some(route.target_memory_uri),
                    layer: Some("l2".to_string()),
                    tokens_used: (summary.len() / 4) as i64,
                    latency_ms: start.elapsed().as_millis() as i64,
                    blocked_by_privacy: false,
                    created_at: Utc::now().timestamp(),
                });
            }
        }
        _ => {}
    }

    Ok(())
}

fn enforce_startup_caps(text: &str, max_lines: usize, max_bytes: usize) -> String {
    let limited_lines: String = text
        .lines()
        .take(max_lines)
        .collect::<Vec<&str>>()
        .join("\n");
    if limited_lines.len() <= max_bytes {
        return limited_lines;
    }
    let mut end = max_bytes;
    while end > 0 && !limited_lines.is_char_boundary(end) {
        end -= 1;
    }
    limited_lines[..end].to_string()
}
