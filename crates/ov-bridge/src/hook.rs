use crate::mcp::{extract_top_hit, truncate_to_token_budget};
use chrono::Utc;
use ov_core::dreaming::db::{DesktopDb, RecallEvent};
use ov_core::ov_client::{FindRequest, OvClient};
use ov_core::policy::model::{AgentId, PolicyConfig};
use ov_core::policy::router::resolve_route_for_agent;
use ov_core::policy::store::PolicyStore;
use ov_core::provenance::wrap_ifc_readonly;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
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
    let db = DesktopDb::open_default().ok();
    Ok(execute_hook(event, agent, payload, &policy, &client, db.as_ref()).await)
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

    let text = match event {
        "session-start" => {
            // Position-Biased Structured Index Header (Displacement: Primacy -> Middle TOC -> Recency)
            let context_text = |res: Value| {
                res.as_str()
                    .or_else(|| res["content"].as_str())
                    .or_else(|| res["text"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| serde_json::to_string(&res).unwrap_or_default())
            };
            let project_text = client
                .read_content(&route.target_resource_uri, budget.default_recall_layer.as_str())
                .await
                .map(context_text)
                .unwrap_or_default();
            let user_mem_l0 = client
                .read_content(&route.target_memory_uri, "l0")
                .await
                .map(context_text)
                .unwrap_or_default();

            let mut sections = Vec::new();
            // 1. Head (Primacy Advantage): Core Scope & Read-Only Rule Constraints
            sections.push(format!(
                "## [Head / Policy & Scope]\n- Active Project Scope: `{}`\n- Shared Memory Root: `{}`\n- Protected Rules (Read-Only): `{}`",
                route.target_resource_uri,
                route.target_memory_uri,
                policy.routing.protected_rule_globs.join(", ")
            ));

            // 2. Middle (Reference TOC): Lazy-load index & optional Agent private namespace
            let mut toc = String::from("## [Middle / Structured Index TOC (Use `read_uri` for L1/L2)]\n");
            if let Some(priv_uri) = &route.agent_private_memory_uri {
                toc.push_str(&format!("- Agent Private Space: `{}`\n", priv_uri));
            }
            if !project_text.trim().is_empty() {
                toc.push_str(&format!("- Project Context ({}):\n{}\n", budget.default_recall_layer.as_str().to_uppercase(), project_text.trim()));
            }
            sections.push(toc);

            // 3. Tail (Recency Advantage): Shared User Preferences / Active Memory Abstract
            if !user_mem_l0.trim().is_empty() {
                sections.push(format!(
                    "## [Tail / Active User Preferences & Recent Context]\n{}",
                    user_mem_l0.trim()
                ));
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
            let Ok(res) = client
                .find(&FindRequest {
                    query: prompt.to_string(),
                    target_uri: Some(route.target_resource_uri).filter(|uri| !uri.is_empty()),
                    limit: Some(5),
                })
                .await
            else {
                return String::new();
            };
            let Some((top_uri, _)) = extract_top_hit(&res) else {
                return String::new();
            };
            let text = truncate_to_token_budget(
                &serde_json::to_string_pretty(&res).unwrap_or_default(),
                budget.max_recall_tokens,
            );
            evt.target_uri = Some(top_uri);
            evt.query = Some(prompt.chars().take(120).collect());
            evt.layer = Some("l0".to_string());
            evt.tokens_used = (text.len() / 4) as i64;
            text
        }
        "stop" => {
            evt.event_type = "hook_stop".to_string();
            evt.target_uri = Some(route.target_memory_uri);
            evt.blocked_by_privacy = !policy.global.capture_enabled;
            if evt.blocked_by_privacy {
                evt.layer = None;
            } else {
                let summary = payload_text(
                    payload,
                    &[
                        "transcript_summary",
                        "transcriptSummary",
                        "last_assistant_message",
                        "lastAssistantMessage",
                    ],
                )
                .map(str::to_owned)
                .or_else(|| {
                    let path = payload_text(payload, &["transcript_path", "transcriptPath"])?;
                    transcript_last_assistant(path)
                });
                let Some(summary) = summary else {
                    return String::new();
                };
                if !commit_summary(client, agent, &route.target_resource_uri, &summary).await {
                    return String::new();
                }
                evt.layer = Some("l2".to_string());
                evt.tokens_used = (summary.len() / 4) as i64;
            }
            String::new()
        }
        _ => return String::new(),
    };

    if let Some(db) = db {
        evt.latency_ms = start.elapsed().as_millis() as i64;
        let _ = db.record_event_for_profile(&client.profile_key(), &evt);
    }
    wrap_ifc_readonly(&text, &format!("{event}:{}", agent.as_str()), "reference")
}

fn payload_text<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter_map(|key| payload.get(*key).and_then(Value::as_str))
        .find(|text| !text.trim().is_empty())
}

async fn commit_summary(client: &OvClient, agent: AgentId, resource_uri: &str, summary: &str) -> bool {
    let Ok(session) = client
        .create_session(json!({ "agent": agent.as_str(), "resource_uri": resource_uri }))
        .await
    else {
        return false;
    };
    let Some(id) = payload_text(&session, &["session_id", "id"]) else {
        return false;
    };
    client
        .add_session_message(id, "assistant", summary)
        .await
        .is_ok()
        && client.commit_session(id).await.is_ok()
}

fn transcript_last_assistant(path: &str) -> Option<String> {
    const TAIL_BYTES: u64 = 256 * 1024;
    let mut file = File::open(path).ok()?;
    let offset = file.metadata().ok()?.len().saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(offset.saturating_sub(1))).ok()?;
    let mut tail = Vec::new();
    file.take(TAIL_BYTES + 1).read_to_end(&mut tail).ok()?;
    // Drop the first record when the window starts in the middle of a JSONL line.
    let start = if offset == 0 {
        0
    } else {
        tail.iter().position(|byte| *byte == b'\n')? + 1
    };
    tail[start..].rsplit(|byte| *byte == b'\n').find_map(|line| {
        let entry: Value = serde_json::from_slice(line).ok()?;
        let message = match entry["type"].as_str() {
            Some("response_item") => &entry["payload"],
            Some("event_msg") if entry["payload"]["type"] == "agent_message" => {
                return payload_text(&entry["payload"], &["message"]).map(str::to_owned);
            }
            Some("assistant") => entry.get("message").unwrap_or(&entry),
            _ => &entry,
        };
        let is_assistant = message["role"].as_str()
            .map_or_else(|| entry["type"] == "assistant", |role| role == "assistant");
        if !is_assistant {
            return None;
        }
        let content = &message["content"];
        let text = if let Some(text) = content.as_str() {
            text.trim().to_string()
        } else {
            content.as_array()?.iter()
                .filter(|block| matches!(block["type"].as_str(), Some("text" | "output_text")))
                .filter_map(|block| block["text"].as_str())
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>().join("\n")
        };
        (!text.is_empty()).then_some(text)
    })
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
