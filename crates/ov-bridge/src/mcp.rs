use chrono::Utc;
use ov_core::dreaming::db::{DesktopDb, RecallEvent, RecallHitRecord};
use ov_core::ov_client::{ContextSearchRequest, FindRequest, OvClient};
use ov_core::policy::model::{AgentId, ContextLayer, MemoryCategory, PolicyConfig};
use ov_core::policy::router::{
    is_foreign_agent_private_uri, is_protected_rule_path, project_peer, resolve_route_for_agent,
};
use ov_core::policy::store::PolicyStore;
use ov_core::provenance::{
    is_expired, is_superseded, render_with_provenance, wrap_ifc_readonly, MemoryProvenance,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::time::Instant;
use uuid::Uuid;

pub async fn run_stdio_server(agent: AgentId) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut stdout = io::stdout();
    let mut line = Vec::new();

    while stdin.read_until(b'\n', &mut line)? != 0 {
        let parsed = serde_json::from_slice(&line);
        line.clear();
        let req: Value = match parsed {
            Ok(v) => v,
            Err(_) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": "Parse error" }
                });
                writeln!(stdout, "{response}")?;
                stdout.flush()?;
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        if id.is_none() {
            continue;
        }

        let response = match method {
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
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
                    "target_uri": { "type": "string", "description": "Defaults to the workspace scope. Use a viking:// URI to override it or an empty string for global recall." },
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
    if std::env::var("OPENVIKING_DISABLE_HOOKS").as_deref() == Ok("1") {
        return Err("OpenViking tools are disabled for this reflector process".to_string());
    }
    let policy = PolicyStore::load_or_init().map_err(|e| e.to_string())?;
    let client = OvClient::from_local_config().map_err(|e| e.to_string())?;
    let db = DesktopDb::open_default().ok();
    call_tool(agent, params, &policy, &client, db.as_ref()).await
}

pub(crate) async fn call_tool(
    agent: AgentId,
    params: &Value,
    policy: &PolicyConfig,
    client: &OvClient,
    db: Option<&DesktopDb>,
) -> Result<String, String> {
    let start = Instant::now();
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let budget = policy.budget.resolve_for(agent);
    let check_access = |uri: &str| {
        if is_foreign_agent_private_uri(policy, agent, uri) {
            Err("Access denied: URI belongs to another agent's private namespace")
        } else {
            Ok(())
        }
    };

    match name {
        "recall" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("Missing query")?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let route = resolve_route_for_agent(policy, &cwd, Some(agent));
            let explicit_uri = args.get("target_uri").and_then(Value::as_str);
            let peer = if explicit_uri.is_none() { project_peer(&route) } else { None };
            let target_uri = explicit_uri.map(|uri| uri.trim().to_owned()).unwrap_or(route.target_resource_uri);
            let target_uri = (!target_uri.is_empty()).then_some(target_uri);
            if let Some(uri) = target_uri.as_deref() {
                check_access(uri)?;
            }
            let limit = args.get("limit").and_then(Value::as_u64)
                .map(|n| usize::try_from(n).unwrap_or(usize::MAX));

            let mut candidates = if limit == Some(0) {
                Ok(Vec::new())
            } else if let Some(uri) = &target_uri {
                // Keep the server's default candidate pool for small combined recall limits.
                let fetch_limit = if peer.is_some() { limit.filter(|&n| n > 10) } else { limit };
                find_scopes(client, query, uri, peer.as_deref(), fetch_limit, None).await
            } else {
                client
                    .search_context(&ContextSearchRequest {
                        query: query.to_string(),
                        target_uri: None,
                        mode: Some("context".to_string()),
                        limit,
                    })
                    .await.map(parse_candidates).map_err(|e| e.to_string())
            }?;

            candidates.retain(|item| !is_foreign_agent_private_uri(policy, agent, &item.uri));
            let (text, layer, hits) = render_recall(
                client, candidates, &budget, &policy.budget.category_quotas,
                policy.lifecycle.archival.respect_ttl_expiry, limit,
            ).await;
            let wrapped = wrap_ifc_readonly(
                &text,
                &format!("recall:{}", agent.as_str()),
                "reference",
            );
            let approx_tokens = (wrapped.len() / 4) as i64;

            if let Some(db) = db {
                let evt = RecallEvent {
                    id: Uuid::new_v4().to_string(),
                    agent_id: agent.as_str().to_string(),
                    event_type: "recall".to_string(),
                    query: Some(query.to_string()),
                    target_uri: hits.iter().find(|h| h.injected).map(|h| h.memory_uri.clone()),
                    layer: Some(layer),
                    tokens_used: approx_tokens,
                    latency_ms: start.elapsed().as_millis() as i64,
                    blocked_by_privacy: false,
                    created_at: Utc::now().timestamp(),
                };
                if let Err(error) = db.record_event_for_profile(&client.profile_key(), &evt)
                    .and_then(|_| db.record_recall_hit_details(&evt.id, &hits)) {
                    eprintln!("OpenViking recall telemetry: {error}");
                }
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
            let route = resolve_route_for_agent(policy, &cwd, Some(agent));
            let base_prefix = if visibility == "private" {
                route
                    .agent_private_memory_uri
                    .as_deref()
                    .unwrap_or(&route.target_memory_uri)
            } else {
                &route.target_resource_uri
            };

            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    let base = base_prefix.trim_end_matches('/');
                    let separator = match base {
                        "viking:" => "//",
                        "" => "",
                        _ => "/",
                    };
                    let suffix = Uuid::new_v4().simple().to_string();
                    format!(
                        "{base}{separator}note-{}-{}.md",
                        Utc::now().format("%Y%m%d-%H%M%S"),
                        &suffix[..4],
                    )
                });

            check_access(&uri)?;
            if !policy.global.capture_enabled {
                if let Some(db) = db {
                    let evt = RecallEvent {
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
                    };
                    let _ = db.record_event_for_profile(&client.profile_key(), &evt);
                }
                return Ok(
                    "Privacy Mode is enabled in OpenViking Desktop; memory write skipped."
                        .to_string(),
                );
            }

            if is_protected_rule_path(policy, &uri) {
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
                let evt = RecallEvent {
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
                };
                let _ = db.record_event_for_profile(&client.profile_key(), &evt);
            }
            Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| "ok".to_string()))
        }
        "read_uri" => {
            let uri = args
                .get("uri")
                .and_then(|v| v.as_str())
                .ok_or("Missing uri")?;
            check_access(uri)?;
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
            check_access(uri)?;
            let mut res = client.ls(uri).await.map_err(|e| e.to_string())?;
            filter_private_entries(&mut res, policy, agent);
            Ok(serde_json::to_string_pretty(&res).unwrap_or_default())
        }
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

#[derive(Debug)]
pub(crate) struct RecallCandidate {
    pub(crate) uri: String,
    pub(crate) score: f64,
    pub(crate) title: String,
    pub(crate) category: Option<MemoryCategory>,
    pub(crate) snippet: String,
    status: Option<String>,
    expires_at: Option<i64>,
}

fn filter_private_entries(value: &mut Value, policy: &PolicyConfig, agent: AgentId) {
    if !policy.routing.isolate_agent_namespaces {
        return;
    }
    match value {
        Value::Array(items) => {
            items.retain(|item| {
                !item["uri"]
                    .as_str()
                    .is_some_and(|uri| is_foreign_agent_private_uri(policy, agent, uri))
            });
            for item in items {
                filter_private_entries(item, policy, agent);
            }
        }
        Value::Object(fields) => {
            for value in fields.values_mut() {
                filter_private_entries(value, policy, agent);
            }
        }
        _ => {}
    }
}

fn detect_known_category(uri: &str, item: &Value) -> Option<MemoryCategory> {
    if let Some(cat) = item.get("category").and_then(Value::as_str) {
        return match cat.to_ascii_lowercase().as_str() {
            "profile" => Some(MemoryCategory::Profile),
            "preferences" | "preference" => Some(MemoryCategory::Preferences),
            "entities" | "entity" => Some(MemoryCategory::Entities),
            "events" | "event" => Some(MemoryCategory::Events),
            "cases" | "case" => Some(MemoryCategory::Cases),
            "patterns" | "pattern" => Some(MemoryCategory::Patterns),
            _ => None,
        };
    }
    let lower = uri.to_ascii_lowercase();
    if lower.contains("/profile/") {
        Some(MemoryCategory::Profile)
    } else if lower.contains("/preferences/") {
        Some(MemoryCategory::Preferences)
    } else if lower.contains("/entities/") {
        Some(MemoryCategory::Entities)
    } else if lower.contains("/events/") {
        Some(MemoryCategory::Events)
    } else if lower.contains("/cases/") {
        Some(MemoryCategory::Cases)
    } else if lower.contains("/patterns/") {
        Some(MemoryCategory::Patterns)
    } else {
        None
    }
}

fn format_candidate(item: &RecallCandidate, include_snippet: bool) -> String {
    let cat = item.category.map(|c| c.as_str()).unwrap_or("general");
    if !include_snippet || item.snippet.is_empty() {
        format!("- [{}] {} (`{}`, score {:.2})", cat, item.title, item.uri, item.score)
    } else {
        format!(
            "### [{}] {} (`{}`, score {:.2})\n{}",
            cat, item.title, item.uri, item.score, item.snippet
        )
    }
}

/// Parse each response once into clean structured candidates (no raw JSON noise).
fn parse_candidates(value: Value) -> Vec<RecallCandidate> {
    let items: Vec<&Value> = if let Some(array) = value.as_array() {
        array.iter().collect()
    } else {
        ["items", "results", "entries", "memories", "resources", "skills"]
            .iter()
            .filter_map(|key| value.get(*key).and_then(Value::as_array))
            .flatten()
            .collect()
    };
    items
        .into_iter()
        .filter_map(|item| {
            let uri = item["uri"]
                .as_str()
                .filter(|uri| !uri.is_empty())?
                .to_owned();
            let score = item
                .get("score")
                .or_else(|| item.get("similarity"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let title = item
                .get("title")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| {
                    uri.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(&uri)
                        .to_owned()
                });
            let raw_snippet = ["abstract", "overview", "snippet", "summary", "content", "text", "description"]
                .iter()
                .filter_map(|k| item.get(*k).and_then(Value::as_str))
                .map(str::trim)
                .find(|text| !text.is_empty())
                .unwrap_or_default();
            let (fm_prov, _) = ov_core::provenance::parse_frontmatter(raw_snippet);
            let status = item["status"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| fm_prov.as_ref().map(|p| p.status.clone()));
            let expires_at = item["expires_at"]
                .as_i64()
                .or_else(|| {
                    let timestamp = item["expires_at"].as_str()?;
                    chrono::DateTime::parse_from_rfc3339(timestamp).ok().map(|dt| dt.timestamp())
                })
                .or_else(|| {
                    let timestamp = fm_prov.as_ref()?.expires_at.as_deref()?;
                    chrono::DateTime::parse_from_rfc3339(timestamp).ok().map(|dt| dt.timestamp())
                });
            let snippet = raw_snippet.chars().take(600).collect::<String>();
            let category = detect_known_category(&uri, item);
            Some(RecallCandidate {
                uri,
                score,
                title,
                category,
                snippet,
                status,
                expires_at,
            })
        })
        .collect()
}

pub(crate) async fn find_scopes(
    client: &OvClient,
    query: &str,
    resource: &str,
    peer: Option<&str>,
    limit: Option<usize>,
    deadline: Option<tokio::time::Instant>,
) -> Result<Vec<RecallCandidate>, String> {
    let find = |request: FindRequest| async move {
        let search = client.find(&request);
        let result = if let Some(deadline) = deadline {
            tokio::time::timeout_at(deadline, search).await.map_err(|_| "request timed out")?
        } else {
            search.await
        };
        result.map_err(|e| e.to_string())
    };
    let primary = FindRequest {
        query: query.to_owned(),
        target_uri: (!resource.is_empty()).then(|| resource.to_owned()),
        limit,
    };
    let Some(peer) = peer else {
        return find(primary).await.map(parse_candidates);
    };
    let memory = FindRequest {
        query: query.to_owned(),
        target_uri: Some(format!("viking://~/peers/{peer}/memories/")),
        limit,
    };
    let (a, b) = tokio::join!(find(primary), find(memory));
    match (a, b) {
        (Ok(a), Ok(b)) => {
            let mut candidates = parse_candidates(a);
            candidates.extend(parse_candidates(b));
            Ok(candidates)
        }
        (Ok(a), Err(_)) => {
            eprintln!("OpenViking peer recall failed");
            Ok(parse_candidates(a))
        }
        (Err(_), Ok(b)) => {
            eprintln!("OpenViking resource recall failed");
            Ok(parse_candidates(b))
        }
        (Err(a), Err(_)) => Err(a),
    }
}

fn rank_candidates(mut candidates: Vec<RecallCandidate>) -> Vec<RecallCandidate> {
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|item| seen.insert(item.uri.clone()));
    candidates
}

pub(crate) fn is_recall_excluded(raw: &str, respect_ttl_expiry: bool, now_ts: i64) -> bool {
    is_superseded(raw) || (respect_ttl_expiry && is_expired(raw, now_ts))
}

#[cfg(test)]
fn filter_candidates(
    candidates: &mut Vec<RecallCandidate>,
    min_score: f64,
    respect_ttl_expiry: bool,
    now_ts: i64,
) {
    let taken = std::mem::take(candidates);
    let (kept, _) = partition_candidates(taken, min_score, respect_ttl_expiry, now_ts);
    *candidates = kept;
}

fn partition_candidates(
    candidates: Vec<RecallCandidate>,
    min_score: f64,
    respect_ttl_expiry: bool,
    now_ts: i64,
) -> (Vec<RecallCandidate>, Vec<RecallHitRecord>) {
    let mut excluded_reasons: std::collections::HashMap<String, &'static str> =
        std::collections::HashMap::new();
    for item in candidates.iter() {
        let status = item.status.as_deref().unwrap_or_default();
        let is_sup = status.eq_ignore_ascii_case("superseded") || is_superseded(&item.snippet);
        let is_exp = respect_ttl_expiry
            && (status.eq_ignore_ascii_case("expired")
                || item.expires_at.is_some_and(|ts| ts <= now_ts)
                || is_expired(&item.snippet, now_ts));
        if is_sup {
            excluded_reasons.insert(item.uri.clone(), "superseded");
        } else if is_exp {
            excluded_reasons.entry(item.uri.clone()).or_insert("expired");
        }
    }

    let ranked_all = rank_candidates(candidates);
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for item in ranked_all {
        if let Some(&reason) = excluded_reasons.get(&item.uri) {
            dropped.push(RecallHitRecord {
                memory_uri: item.uri,
                score: item.score,
                injected: false,
                reason: reason.to_string(),
            });
        } else if item.score < min_score {
            dropped.push(RecallHitRecord {
                memory_uri: item.uri,
                score: item.score,
                injected: false,
                reason: "below_min_score".to_string(),
            });
        } else {
            kept.push(item);
        }
    }
    (kept, dropped)
}

/// Context Assembly II:
/// 1. Rank & deduplicate once by URI while recording exact exclusion reasons.
/// 2. Reserve progressive L1/L2 expansion budget for the top hit BEFORE packing remaining L0 items.
/// 3. Enforce per-category soft quotas (`category_quotas`) and total budget including formatting & IFC overhead.
pub(crate) async fn render_recall(
    client: &OvClient,
    candidates: Vec<RecallCandidate>,
    budget: &ov_core::policy::model::EffectiveBudget,
    category_quotas: &std::collections::HashMap<MemoryCategory, usize>,
    respect_ttl_expiry: bool,
    limit: Option<usize>,
) -> (String, String, Vec<RecallHitRecord>) {
    let now_ts = Utc::now().timestamp();
    let (kept, mut dropped_hits) = partition_candidates(
        candidates,
        budget.min_recall_score as f64,
        respect_ttl_expiry,
        now_ts,
    );
    let mut ranked = kept.into_iter().peekable();
    let ifc_overhead = 240usize;
    let total_max = budget.max_recall_tokens.saturating_mul(4).saturating_sub(ifc_overhead);

    // Pre-fetch and reserve budget for top-hit progressive expansion (L0->L1 or default L1/L2)
    let mut detail_block: Option<(String, String)> = None;
    let mut top_summary_only = false;
    let detail_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while let Some(top) = ranked.peek() {
        let selected = if budget.default_recall_layer == ContextLayer::L0
            && top.score >= budget.auto_expand_l1_threshold as f64
        {
            ContextLayer::L1
        } else {
            budget.default_recall_layer
        };
        if selected != ContextLayer::L0 {
            if let Ok(Ok(value)) = tokio::time::timeout_at(
                detail_deadline,
                client.read_content(&top.uri, selected.as_str()),
            )
            .await
            {
                let content = value
                    .as_str()
                    .or_else(|| value["content"].as_str())
                    .or_else(|| value["text"].as_str())
                    .unwrap_or_default();
                if is_recall_excluded(content, respect_ttl_expiry, now_ts) {
                    if let Some(dropped) = ranked.next() {
                        dropped_hits.push(RecallHitRecord {
                            memory_uri: dropped.uri,
                            score: dropped.score,
                            injected: false,
                            reason: if is_superseded(content) {
                                "superseded".to_string()
                            } else {
                                "expired".to_string()
                            },
                        });
                    }
                    continue;
                }
                let content = content.trim();
                if !content.is_empty() {
                    let heading = if selected == ContextLayer::L1 {
                        "Auto-Expanded L1 Overview"
                    } else {
                        "L2 Detail"
                    };
                    let header = format!(
                        "\n\n--- {heading} ({} | score {:.2}) ---\n",
                        top.uri, top.score
                    );
                    let full_entry_len = format_candidate(top, true).len() + 2;
                    top_summary_only = total_max.saturating_sub(full_entry_len) <= header.len() + 32;
                    let entry_len = if top_summary_only {
                        format_candidate(top, false).len() + 2
                    } else {
                        full_entry_len
                    };
                    let available = total_max.saturating_sub(entry_len);
                    let max_detail = if ranked.len() > 1 {
                        available.min(total_max.saturating_mul(60) / 100)
                    } else {
                        available
                    };
                    if max_detail > header.len() + 32 {
                        let body_cap = max_detail - header.len();
                        let mut end = content.len().min(body_cap);
                        while end > 0 && !content.is_char_boundary(end) {
                            end -= 1;
                        }
                        detail_block = Some((
                            format!("{header}{}", &content[..end]),
                            selected.as_str().to_owned(),
                        ));
                    }
                }
            }
        }
        break;
    }

    let reserved_for_detail = detail_block.as_ref().map(|(d, _)| d.len()).unwrap_or(0);
    let list_budget = total_max.saturating_sub(reserved_for_detail);

    let mut text = String::new();
    let mut hits = Vec::new();
    let mut cat_used: std::collections::HashMap<MemoryCategory, usize> =
        std::collections::HashMap::new();
    let mut injected_count = 0usize;
    let max_items = budget.max_recall_items.max(1);

    for (index, item) in ranked.enumerate() {
        let formatted = format_candidate(&item, index != 0 || !top_summary_only);
        let entry_len = formatted.len() + 2;
        let within_limit = limit.map_or(true, |max| injected_count < max);
        let within_category_quota = match item.category {
            Some(cat) => {
                if let Some(&quota_tokens) = category_quotas.get(&cat) {
                    let quota_bytes = quota_tokens.saturating_mul(4);
                    let used = *cat_used.get(&cat).unwrap_or(&0);
                    used == 0 || used + entry_len <= quota_bytes
                } else {
                    true
                }
            }
            None => true,
        };
        let within_budget = text.len() + entry_len <= list_budget;
        let injected = within_limit && injected_count < max_items && within_category_quota && within_budget;
        let reason = if injected {
            injected_count += 1;
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&formatted);
            if let Some(cat) = item.category {
                *cat_used.entry(cat).or_insert(0) += entry_len;
            }
            "injected"
        } else if !within_limit || injected_count >= max_items {
            "max_items_reached"
        } else if !within_category_quota {
            "category_quota"
        } else {
            "token_budget"
        };
        hits.push(RecallHitRecord {
            memory_uri: item.uri,
            score: item.score,
            injected,
            reason: reason.to_string(),
        });
    }

    let mut layer = "l0".to_owned();
    if let Some((detail, detail_layer)) = detail_block {
        if hits.first().is_some_and(|h| h.injected)
            && text.len() + detail.len() <= total_max
        {
            text.push_str(&detail);
            if let Some(first) = hits.first_mut() {
                first.reason = format!("injected_{}", detail_layer);
            }
            layer = detail_layer;
        }
    }

    hits.extend(dropped_hits);
    (text, layer, hits)
}

#[cfg(test)]
mod recall_tests {
    use super::*;

    #[test]
    fn candidates_rank_once_and_keep_best_duplicate() {
        let candidates = parse_candidates(json!({"memories":[
            {"uri":"viking://resources/a", "score":0.5, "snippet":"old"},
            {"uri":"viking://resources/b", "score":0.8, "snippet":"mid"},
            {"uri":"viking://resources/a", "score":0.9, "snippet":"best"}
        ]}));
        let ranked = rank_candidates(candidates);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].uri, "viking://resources/a");
        assert_eq!(ranked[0].score, 0.9);
        assert_eq!(ranked[0].snippet, "best");
    }

    #[test]
    fn formatted_candidate_strips_raw_json_noise() {
        let candidates = parse_candidates(json!({"memories":[
            {"uri":"viking://~/memories/preferences/git.md", "score":0.92, "title":"Git Rules", "snippet":"Use Conventional Commits", "distance":0.08, "internal_id":"xyz"}
        ]}));
        let rendered = format_candidate(&candidates[0], true);
        assert!(rendered.contains("[preferences] Git Rules"));
        assert!(rendered.contains("Use Conventional Commits"));
        assert!(!rendered.contains("internal_id"));
        assert!(!rendered.contains("distance"));
    }

    #[test]
    fn excluded_uri_is_removed_across_duplicate_copies() {
        let mut candidates = parse_candidates(json!({"memories":[
            {"uri":"viking://~/memories/preferences/old.md", "score":0.95, "status":"superseded", "snippet":"superseded copy"},
            {"uri":"viking://~/memories/preferences/old.md", "score":0.80, "snippet":"plain duplicate without metadata"},
            {"uri":"viking://~/memories/preferences/keep.md", "score":0.85, "snippet":"active memory"}
        ]}));
        filter_candidates(&mut candidates, 0.6, true, Utc::now().timestamp());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].uri, "viking://~/memories/preferences/keep.md");
    }

    #[test]
    fn superseded_long_frontmatter_snippet_is_excluded_before_truncation() {
        let padding = "x".repeat(650);
        let long_fm = format!("---\nstatus: superseded\nnotes: {padding}\n---\nActive-looking body");
        let mut candidates = parse_candidates(json!({"memories":[
            {"uri":"viking://~/memories/preferences/long.md", "score":0.95, "snippet": long_fm}
        ]}));
        filter_candidates(&mut candidates, 0.6, true, Utc::now().timestamp());
        assert!(candidates.is_empty());
    }

    #[test]
    fn partition_candidates_records_exact_skip_reasons() {
        let now = Utc::now().timestamp();
        let candidates = vec![
            RecallCandidate {
                uri: "viking://~/memories/preferences/good.md".into(),
                title: "good".into(),
                snippet: "Use pnpm".into(),
                score: 0.88,
                category: Some(MemoryCategory::Preferences),
                status: Some("active".into()),
                expires_at: None,
            },
            RecallCandidate {
                uri: "viking://~/memories/preferences/old.md".into(),
                title: "old".into(),
                snippet: "---
status: superseded
---
Use yarn".into(),
                score: 0.85,
                category: Some(MemoryCategory::Preferences),
                status: Some("superseded".into()),
                expires_at: None,
            },
            RecallCandidate {
                uri: "viking://~/memories/events/past.md".into(),
                title: "past".into(),
                snippet: "Meeting yesterday".into(),
                score: 0.79,
                category: Some(MemoryCategory::Events),
                status: Some("active".into()),
                expires_at: Some(now - 60),
            },
            RecallCandidate {
                uri: "viking://~/memories/cases/weak.md".into(),
                title: "weak".into(),
                snippet: "Unrelated note".into(),
                score: 0.45,
                category: Some(MemoryCategory::Cases),
                status: Some("active".into()),
                expires_at: None,
            },
        ];
        let (kept, dropped) = partition_candidates(candidates, 0.60, true, now);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].uri, "viking://~/memories/preferences/good.md");
        assert_eq!(dropped.len(), 3);
        let by_uri: std::collections::HashMap<_, _> =
            dropped.into_iter().map(|h| (h.memory_uri, h.reason)).collect();
        assert_eq!(by_uri["viking://~/memories/preferences/old.md"], "superseded");
        assert_eq!(by_uri["viking://~/memories/events/past.md"], "expired");
        assert_eq!(by_uri["viking://~/memories/cases/weak.md"], "below_min_score");
    }
}
