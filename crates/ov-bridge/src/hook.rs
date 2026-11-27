use crate::mcp::{find_scopes, render_recall};
use chrono::Utc;
use ov_core::dreaming::db::{DesktopDb, RecallEvent};
use ov_core::ov_client::OvClient;
use ov_core::policy::model::{AgentId, PolicyConfig};
use ov_core::policy::router::{is_foreign_agent_private_uri, project_peer, resolve_route_for_agent};
use ov_core::policy::store::PolicyStore;
use ov_core::provenance::wrap_ifc_readonly;
use serde_json::{json, Value};
use std::io::{self, Read};
use ov_core::capture::collect;
use ov_core::fs_util::sha256_hex;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

pub async fn run_hook(event: &str, agent: AgentId) -> anyhow::Result<()> {
    if std::env::var("OPENVIKING_DISABLE_HOOKS").as_deref() == Ok("1") {
        if agent == AgentId::Antigravity || event == "stop" {
            println!("{{}}");
        }
        return Ok(());
    }

    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or_else(|_| json!({}));

    let result = local_hook(event, agent, &payload).await;
    if let Some(output) = hook_stdout(event, agent, result.as_deref().unwrap_or_default()) {
        println!("{output}");
    }
    result.map(|_| ())
}

async fn local_hook(event: &str, agent: AgentId, payload: &Value) -> anyhow::Result<String> {
    let Ok(client) = OvClient::from_local_config() else {
        return Ok(String::new());
    };
    let policy = PolicyStore::load_or_init()?;
    let db = match DesktopDb::open_default() {
        Ok(db) => Some(db),
        Err(error) => { eprintln!("OpenViking local database: {error}"); None }
    };
    let result = execute_hook(event, agent, payload, &policy, &client, db.as_ref()).await;
    if policy.global.capture_enabled && matches!(event, "stop" | "pre-compact" | "session-end" | "session-start")
        && db.as_ref().is_some_and(|db| match db.has_capture_work(&client.profile_key()) {
            Ok(ready) => ready,
            Err(error) => { eprintln!("OpenViking capture queue: {error}"); false }
        }) {
        // Short-lived independent worker; the desktop window need not be open.
        if let Ok(exe) = std::env::current_exe() {
            let mut command = ov_core::platform::silent_command(&exe.to_string_lossy());
            command.args(["sync", "--profile", &client.profile_key()])
                .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            if let Err(error) = command.spawn() {
                eprintln!("OpenViking sync worker: {error}");
            }
        }
    }
    Ok(result)
}

pub(crate) fn hook_stdout(event: &str, agent: AgentId, text: &str) -> Option<String> {
    let has_context = matches!(event, "session-start" | "user-prompt-submit" | "pre-invocation")
        && !text.trim().is_empty();
    match agent {
        AgentId::Antigravity if has_context => Some(
            json!({ "injectSteps": [{ "systemMessage": { "systemMessage": text } }] }).to_string(),
        ),
        AgentId::ClaudeCode | AgentId::Codex if has_context && event != "pre-invocation" => {
            let hook_event_name = if event == "session-start" {
                "SessionStart"
            } else {
                "UserPromptSubmit"
            };
            Some(json!({
                "hookSpecificOutput": { "hookEventName": hook_event_name, "additionalContext": text }
            }).to_string())
        }
        AgentId::Pi if has_context && event != "pre-invocation" => Some(text.to_string()),
        AgentId::Pi => None,
        _ => Some("{}".to_string()),
    }
}

pub(crate) async fn execute_hook(
    event: &str,
    agent: AgentId,
    payload: &Value,
    policy: &PolicyConfig,
    client: &OvClient,
    db: Option<&DesktopDb>,
) -> String {
    let start = Instant::now();
    let budget = policy.budget.resolve_for(agent);
    let cwd = payload_text(payload, &["cwd"])
        .or_else(|| {
            ["workspacePaths", "workspace_paths"]
                .iter()
                .filter_map(|key| payload[*key].as_array())
                .flatten()
                .filter_map(Value::as_str)
                .find(|cwd| !cwd.trim().is_empty())
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let route = resolve_route_for_agent(policy, &cwd, Some(agent));
    let peer = project_peer(&route);
    let mut evt = RecallEvent {
        id: Uuid::new_v4().to_string(),
        agent_id: agent.as_str().to_string(),
        event_type: "recall".to_string(),
        query: None,
        target_uri: Some(route.target_resource_uri.clone()),
        layer: Some(budget.default_recall_layer.as_str().to_string()),
        tokens_used: 0,
        latency_ms: 0,
        blocked_by_privacy: false,
        created_at: Utc::now().timestamp(),
    };

    let mut recall_hits: Vec<(String, f64, bool)> = Vec::new();
    let text = match event {
        "session-start" => {
            // Context Assembly I: Concurrent 4-category Startup Profile within a strict 2s timeout
            let context_text = |res: Value| {
                res.as_str()
                    .or_else(|| res["content"].as_str())
                    .or_else(|| res["text"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| serde_json::to_string(&res).unwrap_or_default())
            };
            let peer_mem_uri = peer.as_deref().map(|p| format!("viking://~/peers/{p}/memories/"));
            let (project_text, user_mem_l0, recent_peer_l0) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                async {
                    let project_fut = client.read_content(
                        &route.target_resource_uri,
                        budget.default_recall_layer.as_str(),
                    );
                    let user_fut = client.read_content(&route.target_memory_uri, "l0");
                    let peer_fut = async {
                        if let Some(uri) = peer_mem_uri.as_deref() {
                            client.read_content(uri, "l0").await.ok().map(context_text).unwrap_or_default()
                        } else {
                            String::new()
                        }
                    };
                    let (p_res, u_res, peer_res) = tokio::join!(project_fut, user_fut, peer_fut);
                    (
                        p_res.ok().map(context_text).unwrap_or_default(),
                        u_res.ok().map(context_text).unwrap_or_default(),
                        peer_res,
                    )
                },
            )
            .await
            .unwrap_or_default();

            let mut sections = Vec::new();
            // 1. Project Profile & Policy (Primacy)
            let mut project_sec = format!(
                "## [Project Profile & Policy]\n- Active Project Scope: `{}`\n- Protected Rules (Read-Only): `{}`",
                route.target_resource_uri,
                policy.routing.protected_rule_globs.join(", ")
            );
            if !project_text.trim().is_empty() {
                project_sec.push_str(&format!(
                    "\n- Project Overview ({}):\n{}",
                    budget.default_recall_layer.as_str().to_uppercase(),
                    project_text.trim()
                ));
            }
            sections.push(project_sec);

            // 2. Knowledge Index (Lazy-load TOC)
            let mut toc = format!(
                "## [Knowledge Index (Use `read_uri` for L1/L2)]\n- Shared User Memories: `{}`\n- Project Resources: `{}`",
                route.target_memory_uri, route.target_resource_uri
            );
            if let Some(uri) = peer_mem_uri.as_deref() {
                toc.push_str(&format!("\n- Project Peer Memories: `{uri}`"));
            }
            if let Some(priv_uri) = &route.agent_private_memory_uri {
                toc.push_str(&format!("\n- Agent Private Space: `{priv_uri}`"));
            }
            sections.push(toc);

            let now_ts = chrono::Utc::now().timestamp();
            // 3. Recent Context (only included when actual active peer memory exists)
            if !recent_peer_l0.trim().is_empty()
                && !ov_core::provenance::is_superseded(&recent_peer_l0)
                && !ov_core::provenance::is_expired(&recent_peer_l0, now_ts)
            {
                let rendered = compact_startup_entry(
                    &recent_peer_l0,
                    peer_mem_uri.as_deref().unwrap_or("peer"),
                    policy.lifecycle.dreaming.max_index_entry_bytes,
                );
                sections.push(format!("## [Recent Project Context]\n{}", rendered));
            }

            // 4. User Profile (Recency Advantage)
            if !user_mem_l0.trim().is_empty()
                && !ov_core::provenance::is_superseded(&user_mem_l0)
                && !ov_core::provenance::is_expired(&user_mem_l0, now_ts)
            {
                let rendered = compact_startup_entry(
                    &user_mem_l0,
                    &route.target_memory_uri,
                    policy.lifecycle.dreaming.max_index_entry_bytes,
                );
                sections.push(format!("## [User Profile & Preferences]\n{}", rendered));
            }

            let combined = sections.join("\n\n");
            let capped = enforce_startup_caps(
                &combined,
                budget.startup_max_lines,
                budget.startup_max_bytes,
            );

            evt.event_type = "hook_start".to_string();
            evt.tokens_used = (capped.len() / 4) as i64;
            capped
        }
        "user-prompt-submit" | "pre-invocation" => {
            let Some(prompt) = payload_text(
                payload,
                &["prompt", "message", "lastUserInput", "last_user_input"],
            ) else {
                return String::new();
            };
            let search_query = build_session_aware_query(
                prompt, payload, agent, db.filter(|_| policy.global.capture_enabled),
            );
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut candidates = match find_scopes(
                client, &search_query, &route.target_resource_uri, peer.as_deref(), Some(5), Some(deadline),
            ).await {
                Ok(candidates) => candidates,
                Err(error) => { eprintln!("OpenViking recall: {error}"); return String::new(); }
            };
            candidates.retain(|item| !is_foreign_agent_private_uri(policy, agent, &item.uri));
            let (text, layer, hits) = render_recall(client, candidates, &budget, &policy.budget.category_quotas, Some(5)).await;
            evt.target_uri = hits.iter().find(|(_,_,injected)| *injected).map(|(uri,_,_)| uri.clone());
            recall_hits = hits;
            evt.query = Some(prompt.chars().take(120).collect());
            evt.layer = Some(layer);
            evt.tokens_used = (text.len() / 4) as i64;
            text
        }
        "stop" | "pre-compact" | "session-end" => {
            evt.event_type = "hook_stop".to_string();
            evt.target_uri = Some(route.target_memory_uri);
            evt.blocked_by_privacy = !policy.global.capture_enabled;
            if evt.blocked_by_privacy {
                evt.layer = None;
                let (session, messages) = collect(payload);
                if let (Some(db), Some(runtime_id)) = (
                    db,
                    session.or_else(|| messages.first().map(|m| m.key.clone())),
                ) {
                    let session_key = sha256_hex(&format!(
                        "{}:{}:{}",
                        agent.as_str(),
                        route.target_resource_uri,
                        runtime_id
                    ));
                    if let Err(error) = db.skip_capture_for_route(
                        &client.profile_key(),
                        &session_key,
                        &privacy_session_key(agent, &runtime_id),
                        agent.as_str(),
                        &route.target_resource_uri,
                        peer.as_deref(),
                        &messages,
                    ) {
                        eprintln!("OpenViking privacy watermark: {error}");
                    }
                }
            } else {
                let (session, messages) = collect(payload);
                let Some(db) = db else { return String::new(); };
                let Some(runtime_id) = session.or_else(|| messages.first().map(|m|m.key.clone())) else {
                    return String::new();
                };
                let flush = matches!(event,"pre-compact" | "session-end");
                let session_key = sha256_hex(&format!("{}:{}:{}", agent.as_str(), route.target_resource_uri, runtime_id));
                match db.enqueue_capture_for_route(
                    &client.profile_key(), &session_key, &privacy_session_key(agent, &runtime_id),
                    agent.as_str(), &route.target_resource_uri, peer.as_deref(), &messages, flush,
                ) {
                    Ok(0) => return String::new(),
                    Ok(_) => {},
                    Err(error) => {
                        eprintln!("OpenViking capture enqueue: {error}");
                        return String::new();
                    }
                }
                evt.event_type = "capture_queued".to_string();
                evt.layer = Some("l2".to_string());
                evt.tokens_used = messages.iter().map(|m| (m.content.len() / 4) as i64).sum();
            }
            String::new()
        }
        _ => return String::new(),
    };

    if let Some(db) = db {
        evt.latency_ms = start.elapsed().as_millis() as i64;
        if let Err(error) = db.record_event_for_profile(&client.profile_key(), &evt) {
            eprintln!("OpenViking telemetry: {error}");
        } else if !recall_hits.is_empty() {
            if let Err(error) = db.record_recall_hits(&evt.id, &recall_hits) {
                eprintln!("OpenViking recall hits: {error}");
            }
        }
    }
    wrap_ifc_readonly(&text, &format!("{event}:{}", agent.as_str()), "reference")
}

fn payload_text<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| payload.get(*key).and_then(Value::as_str))
        .find(|text| !text.trim().is_empty())
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
    let notice = format!("\n\n[Truncated by OpenViking startup byte cap: {max_bytes} bytes]");
    let notice = if notice.len() < max_bytes { notice.as_str() } else { "" };
    let mut end = max_bytes - notice.len();
    while end > 0 && !limited_lines.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{notice}", &limited_lines[..end])
}


/// When the current prompt is short/context-dependent (e.g. "继续", "fix that test"),
/// enrich the search query with up to 2 recent user turns from the transcript/messages
/// without calling any LLM.
fn privacy_session_key(agent: AgentId, runtime_id: &str) -> String {
    sha256_hex(&format!("{}:{runtime_id}", agent.as_str()))
}

fn build_session_aware_query(
    prompt: &str, payload: &Value, agent: AgentId, db: Option<&DesktopDb>,
) -> String {
    let trimmed = prompt.trim();
    let Some(db) = db else { return trimmed.to_owned(); };
    if trimmed.chars().count() >= 40 {
        return trimmed.to_owned();
    }
    let (session, messages) = collect(payload);
    let Some(runtime_id) = session.as_deref().or_else(|| messages.first().map(|m| m.key.as_str())) else {
        return trimmed.to_owned();
    };
    let session = privacy_session_key(agent, runtime_id);
    let recent_user: Vec<&str> = messages
        .iter()
        .rev()
        .filter(|m| m.role == "user")
        .filter(|m| match db.is_privacy_excluded(&session, &m.key) {
            Ok(excluded) => !excluded,
            Err(error) => { eprintln!("OpenViking privacy watermark: {error}"); false }
        })
        .map(|m| m.content.trim())
        .filter(|c| !c.is_empty() && *c != trimmed)
        .take(2)
        .collect();
    if recent_user.is_empty() {
        return trimmed.to_owned();
    }
    let mut ctx = String::new();
    for msg in recent_user.into_iter().rev() {
        if !ctx.is_empty() {
            ctx.push_str(" | ");
        }
        ctx.push_str(msg);
    }
    let capped_ctx: String = ctx.chars().take(300).collect();
    format!("{capped_ctx}\n{trimmed}")
}

fn compact_startup_entry(raw: &str, uri: &str, max_entry_bytes: usize) -> String {
    let trimmed = raw.trim();
    if max_entry_bytes == 0 || trimmed.len() <= max_entry_bytes {
        return trimmed.to_string();
    }
    let first_line = trimmed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "---")
        .unwrap_or("Memory summary");
    let preview: String = first_line.chars().take(120).collect();
    format!("- {} (index link: `{}` — use `read_uri` for full details)", preview, uri)
}
