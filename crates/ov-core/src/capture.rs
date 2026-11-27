//! Incremental, profile-scoped capture using the existing desktop SQLite database.
use crate::dreaming::db::{DesktopDb, RecallEvent};
use crate::fs_util::sha256_hex;
use crate::ov_client::{OvClient, OvClientError};
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedMessage {
    pub key: String,
    pub role: String,
    pub content: String,
}

fn text(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().filter_map(|key| payload.get(*key).and_then(Value::as_str))
        .find(|value| !value.trim().is_empty()).map(str::to_owned)
}

/// Strip injected `<openviking_memory_reference>` blocks to prevent echo-chamber re-capture.
/// Per project design, pre-ingest secret scrubbing is intentionally omitted; use Privacy Mode instead.
fn sanitize(input: &str) -> String {
    let mut remaining = input;
    let mut clean = String::new();
    while let Some(start) = remaining.find("<openviking_memory_reference") {
        clean.push_str(&remaining[..start]);
        let Some(end) = remaining[start..].find("</openviking_memory_reference>") else {
            remaining = "";
            break;
        };
        remaining = &remaining[start + end + "</openviking_memory_reference>".len()..];
    }
    clean.push_str(remaining);
    clean.trim().chars().take(12000).collect()
}

fn message(entry: &Value, ordinal: usize) -> Option<CapturedMessage> {
    let item = match entry["type"].as_str() {
        Some("response_item") => &entry["payload"],
        Some("message" | "assistant" | "user") => entry.get("message").unwrap_or(entry),
        Some("event_msg") => return None, // Codex also emits response_item; avoid double capture.
        _ => entry,
    };
    let role = item["role"].as_str().or_else(|| entry["type"].as_str())?;
    if !matches!(role, "user" | "assistant") {
        return None;
    }
    let body = &item["content"];
    let raw = if let Some(content) = body.as_str() {
        content.to_owned()
    } else {
        body.as_array()?.iter()
            .filter(|part| matches!(part["type"].as_str(), Some("text" | "input_text" | "output_text")))
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>().join("\n")
    };
    let content = sanitize(&raw);
    if content.is_empty() {
        return None;
    }
    let identity = text(entry, &["uuid", "id"])
        .or_else(|| text(item, &["id"]))
        .unwrap_or_else(|| format!("{ordinal}:{}", sha256_hex(&format!("{role}:{raw}"))));
    Some(CapturedMessage { key: sha256_hex(&identity), role: role.to_owned(), content })
}

/// Use stable runtime IDs when available; transcript path is the fallback for resume.
pub fn collect(payload: &Value) -> (Option<String>, Vec<CapturedMessage>) {
    let path = text(payload, &["transcript_path", "transcriptPath"]);
    let session = text(payload, &["session_id", "sessionId", "conversationId"])
        .or_else(|| path.clone());
    if let Some(file) = path.and_then(|path| std::fs::File::open(path).ok()) {
        let messages = BufReader::new(file).lines().enumerate()
            .map_while(|(ordinal, line)| line.ok().map(|line| (ordinal, line)))
            .filter_map(|(ordinal, line)| serde_json::from_str::<Value>(&line).ok()
                .and_then(|entry| message(&entry, ordinal)))
            .collect::<Vec<_>>();
        if !messages.is_empty() {
            return (session, messages);
        }
    }
    if let Some(messages) = payload["messages"].as_array() {
        return (session, messages.iter().enumerate()
            .filter_map(|(ordinal, entry)| message(entry, ordinal)).collect());
    }
    let messages = text(payload, &["transcript_summary", "transcriptSummary",
        "last_assistant_message", "lastAssistantMessage"])
        .and_then(|body| message(&json!({"role":"assistant","content":body}), 0))
        .into_iter().collect();
    (session, messages)
}

#[derive(Debug, Serialize, Default)]
pub struct CaptureHealth {
    pub pending: i64,
    pub sent: i64,
    pub uncertain_sessions: i64,
    pub extraction_tasks: Vec<(String, String)>,
    pub last_error: Option<String>,
}

#[derive(Debug)]
struct Session {
    key: String,
    agent: String,
    resource_uri: String,
    remote_id: Option<String>,
    peer_id: Option<String>,
    needs_reconciliation: bool,
}

impl DesktopDb {
    /// The unique message key is the durable capture watermark. Enqueueing is atomic.
    pub fn enqueue_capture(&self, profile: &str, session: &str, agent: &str,
        uri: &str, peer: Option<&str>, messages: &[CapturedMessage], flush: bool) -> Result<usize,String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        tx.execute("INSERT OR IGNORE INTO capture_sessions(profile_key,session_key,agent,resource_uri,peer_id) VALUES (?1,?2,?3,?4,?5)",
            params![profile,session,agent,uri,peer]).map_err(|e| e.to_string())?;
        if flush {
            tx.execute("UPDATE capture_sessions SET flush_requested=1 WHERE profile_key=?1 AND session_key=?2",
                params![profile,session]).map_err(|e| e.to_string())?;
        }
        let mut added = 0;
        for msg in messages {
            added += tx.execute("INSERT OR IGNORE INTO capture_outbox(profile_key,session_key,event_key,role,content) VALUES (?1,?2,?3,?4,?5)",
                params![profile,session,msg.key,msg.role,msg.content]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(added)
    }

    fn claim_capture(&self, profile: &str, threshold: u64, reconcile: bool) -> Result<Option<Session>, String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
        // A crashed process may have completed a remote write. Never blindly replay it.
        let now = Utc::now().timestamp();
        tx.execute("UPDATE capture_sessions SET state='unknown',last_error='Interrupted sync; reconcile remote session before retry' WHERE profile_key=?1 AND state='sending' AND claimed_at<?2",
            params![profile,now-3600]).map_err(|e| e.to_string())?;
        let session = tx.query_row("SELECT session_key,agent,resource_uri,remote_id,peer_id,state FROM capture_sessions s
            WHERE profile_key=?1 AND (state='idle' OR (?3 AND state='unknown')) AND EXISTS (SELECT 1 FROM capture_outbox o
                WHERE o.profile_key=s.profile_key AND o.session_key=s.session_key AND o.state IN ('pending','sent'))
            AND (s.flush_requested=1 OR ?2=0 OR EXISTS (SELECT 1 FROM capture_outbox o
                WHERE o.profile_key=s.profile_key AND o.session_key=s.session_key AND o.state='sent')
                OR (SELECT COALESCE(SUM(length(CAST(content AS BLOB))),0) FROM capture_outbox o
                WHERE o.profile_key=s.profile_key AND o.session_key=s.session_key AND o.state='pending') >= ?2)
            ORDER BY (SELECT MIN(seq) FROM capture_outbox o WHERE o.profile_key=s.profile_key AND o.session_key=s.session_key AND o.state IN ('pending','sent')) LIMIT 1",
            params![profile,threshold.min(i64::MAX as u64 / 4) as i64 * 4,reconcile], |r| Ok(Session {key:r.get(0)?,agent:r.get(1)?,resource_uri:r.get(2)?,remote_id:r.get(3)?,peer_id:r.get(4)?,needs_reconciliation:r.get::<_,String>(5)?=="unknown"}))
            .optional().map_err(|e| e.to_string())?;
        if let Some(s) = &session {
            tx.execute("UPDATE capture_sessions SET state='sending',claimed_at=?3 WHERE profile_key=?1 AND session_key=?2",
                params![profile,s.key,now]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(session)
    }

    pub fn has_capture_work(&self, profile: &str) -> Result<bool, String> {
        self.lock_conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM capture_sessions s JOIN capture_outbox o
             ON s.profile_key=o.profile_key AND s.session_key=o.session_key
             WHERE s.profile_key=?1 AND s.state='idle' AND o.state IN ('pending','sent'))",
            [profile], |row| row.get(0)).map_err(|e| e.to_string())
    }

    fn capture_rows(&self, profile: &str, session: &str) -> Result<Vec<(i64,String,String)>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT seq,role,content FROM capture_outbox WHERE profile_key=?1 AND session_key=?2 AND state='pending' ORDER BY seq")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![profile,session], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())
    }

    fn capture_state(&self, profile: &str, session: &str, state: &str, error: Option<&str>) -> Result<(), String> {
        self.lock_conn()?.execute("UPDATE capture_sessions SET state=?3,last_error=?4 WHERE profile_key=?1 AND session_key=?2",
            params![profile,session,state,error]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn capture_health(&self, profile: &str) -> Result<CaptureHealth,String> {
        let conn = self.lock_conn()?;
        let (pending,sent) = conn.query_row("SELECT COUNT(CASE WHEN state='pending' THEN 1 END), COUNT(CASE WHEN state='sent' THEN 1 END) FROM capture_outbox WHERE profile_key=?1",
            [profile], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?;
        let uncertain_sessions = conn.query_row("SELECT COUNT(*) FROM capture_sessions WHERE profile_key=?1 AND state='unknown'", [profile], |r| r.get(0)).map_err(|e| e.to_string())?;
        let last_error = conn.query_row("SELECT last_error FROM capture_sessions WHERE profile_key=?1 AND last_error IS NOT NULL ORDER BY claimed_at DESC LIMIT 1", [profile], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT task_id,status FROM capture_tasks WHERE profile_key=?1 ORDER BY rowid DESC LIMIT 50").map_err(|e| e.to_string())?;
        let tasks = stmt.query_map([profile], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?;
        let extraction_tasks = tasks.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        Ok(CaptureHealth {pending,sent,uncertain_sessions,extraction_tasks,last_error})
    }
}

fn write_rejected(error: &OvClientError) -> bool {
    match error {
        OvClientError::Http(e) => e.is_connect(),
        OvClientError::Api { status, .. } => matches!(*status, 400 | 401 | 403 | 404 | 405 | 413 | 415 | 422 | 429),
        _ => false,
    }
}

/// Manual sync can resume an interrupted, exclusively Hub-owned session after
/// the previous request has settled. At most one append was unacknowledged.
async fn reconcile_session(db: &DesktopDb, client: &OvClient, profile: &str,
    session: &Session) -> Result<Option<String>, String> {
    let remote = session.remote_id.as_deref().unwrap_or(&session.key);
    let local_count: i64 = db.lock_conn()?.query_row(
        "SELECT COUNT(*) FROM capture_outbox WHERE profile_key=?1 AND session_key=?2 AND state IN ('sent','committed')",
        params![profile,session.key], |row| row.get(0)).map_err(|e| e.to_string())?;
    let info = match client.get_session(remote).await {
        Ok(info) => info,
        Err(OvClientError::Api {status:404,..}) if session.remote_id.is_none() && local_count==0 => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let count = info["total_message_count"].as_i64()
        .ok_or("Remote session has no total_message_count; cannot reconcile automatically")?;
    if count == local_count + 1 {
        let changed = db.lock_conn()?.execute(
            "UPDATE capture_outbox SET state='sent' WHERE seq=(SELECT MIN(seq) FROM capture_outbox
             WHERE profile_key=?1 AND session_key=?2 AND state='pending')",
            params![profile,session.key]).map_err(|e| e.to_string())?;
        if changed != 1 { return Err("Remote session count differs from local capture".into()); }
    } else if count != local_count {
        return Err("Remote session count differs from local capture".into());
    }
    let id = text(&info, &["session_id"]).unwrap_or_else(|| remote.to_owned());
    db.lock_conn()?.execute("UPDATE capture_sessions SET remote_id=?3 WHERE profile_key=?1 AND session_key=?2",
        params![profile,session.key,id]).map_err(|e| e.to_string())?;
    Ok(Some(id))
}

/// Refresh only within one overall time budget, not a timeout per task.
pub async fn refresh_extraction_tasks(db: &DesktopDb, client: &OvClient) -> Result<(), String> {
    let profile = client.profile_key();
    let tasks = db.capture_health(&profile)?.extraction_tasks;
    let refresh = async {
        for (task, status) in tasks {
            if matches!(status.as_str(), "completed" | "failed" | "cancelled") { continue; }
            let Ok(result) = client.get_task(&task).await else { break; };
            if let Some(status) = result["status"].as_str() {
                db.lock_conn()?.execute("UPDATE capture_tasks SET status=?3 WHERE profile_key=?1 AND task_id=?2",
                    params![profile,task,status]).map_err(|e| e.to_string())?;
            }
        }
        Ok::<(),String>(())
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), refresh).await.unwrap_or(Ok(()))
}

/// Checkpoint each acknowledged append. Retrying commit never replays messages.
pub async fn sync_capture(db: &DesktopDb, client: &OvClient, threshold: u64, reconcile: bool) -> Result<usize,String> {
    let profile = client.profile_key();
    let mut committed = 0;
    while let Some(mut session) = db.claim_capture(&profile,threshold,reconcile)? {
        if session.needs_reconciliation {
            match reconcile_session(db,client,&profile,&session).await {
                Ok(id) => session.remote_id = id,
                Err(error) => {
                    db.capture_state(&profile,&session.key,"unknown",Some("Remote session needs reconciliation"))?;
                    return Err(error);
                }
            }
        }
        let remote = match session.remote_id {
            Some(id) => id,
            None => {
                let mut config = json!({"session_id":session.key,"auto_commit_policy":null});
                if session.peer_id.is_some() {
                    config["memory_policy"] = json!({"self":{"enabled":true},"peer":{"enabled":true}});
                }
                match client.create_session(config).await {
                Ok(result) => {
                    let Some(id) = text(&result,&["session_id","id"]) else {
                        db.capture_state(&profile,&session.key,"unknown",Some("Session creation returned no ID"))?;
                        return Err("Session creation returned no ID".into());
                    };
                    db.lock_conn()?.execute("UPDATE capture_sessions SET remote_id=?3 WHERE profile_key=?1 AND session_key=?2",
                        params![profile,session.key,id]).map_err(|e| e.to_string())?;
                    id
                },
                Err(error) => {
                    db.capture_state(&profile,&session.key,if write_rejected(&error) {"idle"} else {"unknown"},Some("Session creation failed"))?;
                    return Err(error.to_string());
                }
                }
            }
        };
        for (seq,role,content) in db.capture_rows(&profile,&session.key)? {
            if let Err(error) = client.add_session_message_for_peer(&remote,&role,&content,session.peer_id.as_deref()).await {
                db.capture_state(&profile,&session.key,if write_rejected(&error) {"idle"} else {"unknown"},Some("Message delivery failed"))?;
                return Err(error.to_string());
            }
            db.lock_conn()?.execute("UPDATE capture_outbox SET state='sent' WHERE seq=?1",[seq]).map_err(|e| e.to_string())?;
        }
        match client.commit_session(&remote).await {
            Ok(result) => {
                let task = text(&result,&["task_id"]);
                let mut conn = db.lock_conn()?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
                tx.execute("UPDATE capture_outbox SET state='committed',content='' WHERE profile_key=?1 AND session_key=?2 AND state='sent'",
                    params![profile,session.key]).map_err(|e| e.to_string())?;
                if let Some(task_id) = &task {
                    tx.execute("INSERT OR IGNORE INTO capture_tasks(profile_key,task_id) VALUES (?1,?2)",
                        params![profile,task_id]).map_err(|e| e.to_string())?;
                }
                tx.execute("UPDATE capture_sessions SET state='idle',last_error=NULL,
                    flush_requested=CASE WHEN EXISTS (SELECT 1 FROM capture_outbox o WHERE o.profile_key=?1 AND o.session_key=?2 AND o.state='pending') THEN flush_requested ELSE 0 END
                    WHERE profile_key=?1 AND session_key=?2",
                    params![profile,session.key]).map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
                drop(conn);
                db.record_event_for_profile(&profile, &RecallEvent {
                    id: uuid::Uuid::new_v4().to_string(), agent_id: session.agent.clone(),
                    event_type: "capture".into(), query: None, target_uri: Some(session.resource_uri.clone()),
                    layer: Some("l2".into()), tokens_used: 0, latency_ms: 0,
                    blocked_by_privacy: false, created_at: Utc::now().timestamp()
                })?;
                committed += 1;
            }
            Err(error) => {
                // OpenViking archives only live messages; a repeated empty commit is a no-op.
                db.capture_state(&profile,&session.key,"idle",Some("Commit failed; retry sync"))?;
                return Err(error.to_string());
            }
        }
    }
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collects_user_and_assistant_stripping_injected_references() {
        let payload = json!({"session_id":"one","messages":[
            {"role":"user","content":"Fix payment\nUse Conventional Commits"},
            {"role":"assistant","content":[{"type":"text","text":"<openviking_memory_reference>old</openviking_memory_reference>\nFixed"},{"type":"thinking","text":"internal"}]}
        ]});
        let (session,messages) = collect(&payload);
        assert_eq!(session.as_deref(),Some("one"));
        assert_eq!(messages.len(),2);
        assert_eq!(messages[0].content,"Fix payment\nUse Conventional Commits");
        assert_eq!(messages[1].content,"Fixed");
    }

    #[test]
    fn enqueue_is_incremental_and_profile_scoped() {
        let db = DesktopDb::open_in_memory().unwrap();
        let (_,messages) = collect(&json!({"messages":[{"role":"user","content":"one"},{"role":"assistant","content":"two"}]}));
        assert_eq!(db.enqueue_capture("a","session","codex","viking://resources/p/",None,&messages,false).unwrap(),2);
        assert_eq!(db.enqueue_capture("a","session","codex","viking://resources/p/",None,&messages,false).unwrap(),0);
        assert_eq!(db.enqueue_capture("b","session","codex","viking://resources/p/",None,&messages,false).unwrap(),2);
        assert_eq!(db.capture_health("a").unwrap().pending,2);
        assert_eq!(db.capture_health("b").unwrap().pending,2);
    }
}
