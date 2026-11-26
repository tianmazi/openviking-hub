use chrono::Utc;
use ov_core::dreaming::db::{DesktopDb, RecallEvent};
use ov_core::ov_client::{ContextSearchRequest, OvClient};
use ov_core::policy::model::{AgentId, ContextLayer};
use ov_core::policy::router::{is_protected_rule_path, resolve_route_for_agent};
use ov_core::policy::store::PolicyStore;
use ov_core::provenance::{render_with_provenance, wrap_ifc_readonly, MemoryProvenance};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::time::Instant;
use uuid::Uuid;

pub async fn run_stdio_server(agent: AgentId) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        if id.is_none() {
            continue;
        }

        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "openviking-desktop-bridge",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_definitions()
                }
            }),
            "tools/call" => {
                let res = handle_tool_call(agent, &params).await;
                match res {
                    Ok(content) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": content }]
                        }
                    }),
                    Err(err) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "isError": true,
                            "content": [{ "type": "text", "text": err }]
                        }
                    }),
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("Method not found: {}", method) }
            }),
        };

        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "recall",
            "description": "Search shared OpenViking memories and project resources with policy-governed L0/L1/L2 token budgeting and automatic L0->L1 expansion.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "target_uri": { "type": "string", "description": "Optional viking:// scope override" },
                    "limit": { "type": "integer", "description": "Maximum items to return" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "remember",
            "description": "Persist a durable user preference, project decision, or note into OpenViking with YAML Frontmatter provenance (respects Privacy Mode, namespace isolation, and protected rule globs).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "uri": { "type": "string", "description": "Optional target viking:// URI (auto-routed if omitted)" },
                    "content": { "type": "string", "description": "Markdown content to store" },
                    "visibility": { "type": "string", "enum": ["shared", "private"], "description": "Use 'private' to isolate into this agent's private namespace when enabled" },
                    "verified": { "type": "boolean", "description": "Optional verification flag for Write-Gate provenance" }
                },
                "required": ["content"]
            }
        },
        {
            "name": "read_uri",
            "description": "Read a viking:// entry at L0 (abstract), L1 (overview), or L2 (full content).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "uri": { "type": "string" },
                    "layer": { "type": "string", "enum": ["l0", "l1", "l2"] }
                },
                "required": ["uri"]
            }
        },
        {
            "name": "list_uri",
            "description": "List files and directories under a viking:// virtual filesystem URI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "uri": { "type": "string" }
                },
                "required": ["uri"]
            }
        }
    ])
}

async fn handle_tool_call(agent: AgentId, params: &Value) -> Result<String, String> {
    let start = Instant::now();
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let policy = PolicyStore::load_or_init().map_err(|e| e.to_string())?;
    let budget = policy.budget.resolve_for(agent);
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    let db = DesktopDb::open_default().ok();

    match name {
        "recall" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("Missing query")?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let route = resolve_route_for_agent(&policy, &cwd, Some(agent));
            let target_uri = args
                .get("target_uri")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or(Some(route.target_resource_uri.clone()));
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);

            let res = client
                .search_context(&ContextSearchRequest {
                    query: query.to_string(),
                    target_uri: target_uri.clone(),
                    mode: Some("context".to_string()),
                    limit,
                })
                .await
                .map_err(|e| e.to_string())?;

            let mut effective_layer = budget.default_recall_layer;
            let mut combined = serde_json::to_string_pretty(&res).unwrap_or_default();

            // Progressive L0 -> L1 auto-expansion when top hit exceeds threshold
            if effective_layer == ContextLayer::L0 {
                if let Some((top_uri, top_score)) = extract_top_hit(&res) {
                    if top_score >= budget.auto_expand_l1_threshold as f64 {
                        if let Ok(l1_val) = client.read_content(&top_uri, "l1").await {
                            let l1_text = l1_val
                                .as_str()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string_pretty(&l1_val).unwrap_or_default()
                                });
                            combined.push_str(&format!(
                                "\n\n--- Auto-Expanded L1 Overview ({} | score {:.2}) ---\n{}",
                                top_uri, top_score, l1_text
                            ));
                            effective_layer = ContextLayer::L1;
                        }
                    }
                }
            }

            let budgeted = truncate_to_token_budget(&combined, budget.max_recall_tokens);
            let wrapped = wrap_ifc_readonly(
                &budgeted,
                &format!("recall:{}", agent.as_str()),
                "reference",
            );
            let approx_tokens = (wrapped.len() / 4) as i64;

            if let Some(db) = db {
                let _ = db.record_event(&RecallEvent {
                    id: Uuid::new_v4().to_string(),
                    agent_id: agent.as_str().to_string(),
                    event_type: "recall".to_string(),
                    query: Some(query.to_string()),
                    target_uri,
                    layer: Some(effective_layer.as_str().to_string()),
                    tokens_used: approx_tokens,
                    latency_ms: start.elapsed().as_millis() as i64,
                    blocked_by_privacy: false,
                    created_at: Utc::now().timestamp(),
                });
            }
            Ok(wrapped)
        }
        "remember" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("Missing content")?;
            let visibility = args
                .get("visibility")
                .and_then(|v| v.as_str())
                .unwrap_or("shared");
            let verified = args
                .get("verified")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let cwd = std::env::current_dir().unwrap_or_default();
            let route = resolve_route_for_agent(&policy, &cwd, Some(agent));
            let base_prefix = if visibility == "private" {
                route
                    .agent_private_memory_uri
                    .clone()
                    .unwrap_or_else(|| route.target_memory_uri.clone())
            } else {
                route.target_resource_uri.clone()
            };

            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "{}note-{}.md",
                        base_prefix,
                        Utc::now().format("%Y%m%d-%H%M%S")
                    )
                });

            if !policy.global.capture_enabled {
                if let Some(db) = db {
                    let _ = db.record_event(&RecallEvent {
                        id: Uuid::new_v4().to_string(),
                        agent_id: agent.as_str().to_string(),
                        event_type: "capture".to_string(),
                        query: None,
                        target_uri: Some(uri.clone()),
                        layer: Some("l2".to_string()),
                        tokens_used: 0,
                        latency_ms: start.elapsed().as_millis() as i64,
                        blocked_by_privacy: true,
                        created_at: Utc::now().timestamp(),
                    });
                }
                return Ok(
                    "Privacy Mode is enabled in OpenViking Desktop; memory write skipped."
                        .to_string(),
                );
            }

            if is_protected_rule_path(&policy, &uri) {
                return Err(format!(
                    "Write denied: '{}' is protected by routing.protected_rule_globs.",
                    uri
                ));
            }

            let scope_label = if uri.starts_with("viking://agent/") {
                "agent_private"
            } else if visibility == "private" {
                "private"
            } else {
                "shared"
            };
            let prov = MemoryProvenance::new(agent.as_str(), scope_label, verified, 0.85);
            let enriched_content = render_with_provenance(content, &prov);

            let res = client
                .write_content(&uri, &enriched_content)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(db) = db {
                let _ = db.record_event(&RecallEvent {
                    id: Uuid::new_v4().to_string(),
                    agent_id: agent.as_str().to_string(),
                    event_type: "capture".to_string(),
                    query: None,
                    target_uri: Some(uri.clone()),
                    layer: Some("l2".to_string()),
                    tokens_used: (enriched_content.len() / 4) as i64,
                    latency_ms: start.elapsed().as_millis() as i64,
                    blocked_by_privacy: false,
                    created_at: Utc::now().timestamp(),
                });
            }
            Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| "ok".to_string()))
        }
        "read_uri" => {
            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or("Missing uri")?;
            let layer = args
                .get("layer")
                .and_then(|v| v.as_str())
                .unwrap_or(budget.default_recall_layer.as_str());
            let res = client
                .read_content(uri, layer)
                .await
                .map_err(|e| e.to_string())?;
            let raw = serde_json::to_string_pretty(&res).unwrap_or_default();
            Ok(wrap_ifc_readonly(
                &raw,
                &format!("read_uri:{}", uri),
                "reference",
            ))
        }
        "list_uri" => {
            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .unwrap_or("viking://");
            let res = client.ls(uri).await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

fn extract_top_hit(val: &Value) -> Option<(String, f64)> {
    let mut slices: Vec<&[Value]> = Vec::new();
    if let Some(items) = val.as_array() {
        slices.push(items);
    } else {
        for key in ["items", "results", "memories", "resources", "skills"] {
            if let Some(items) = val.get(key).and_then(|i| i.as_array()) {
                slices.push(items);
            }
        }
    }
    let mut best: Option<(String, f64)> = None;
    for items in slices {
        for item in items {
            let Some(uri) = item.get("uri").and_then(|u| u.as_str()) else {
                continue;
            };
            let score = item
                .get("score")
                .or_else(|| item.get("similarity"))
                .and_then(|s| s.as_f64())
                .unwrap_or(0.0);
            if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                best = Some((uri.to_string(), score));
            }
        }
    }
    best
}

pub fn truncate_to_token_budget(text: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(4);
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut end = max_chars;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n\n[Truncated by OpenViking Desktop policy budget: {} tokens]",
        &text[..end],
        max_tokens
    )
}
