// Exercise the same handlers as the binary with an isolated policy and database.
#[allow(dead_code)]
#[path = "../src/hook.rs"]
mod hook;
#[allow(dead_code)]
#[path = "../src/mcp.rs"]
mod mcp;

use ov_core::dreaming::db::DesktopDb;
use ov_core::dreaming::engine::{apply_accepted_changes, rollback_dreaming_run, run_dreaming};
use ov_core::ov_client::{OvCliConfig, OvClient};
use ov_core::policy::model::{
    AgentId, ChangeAction, PolicyConfig, ReflectorMode, ReviewStatus, RiskLevel, VisibilityScope, WorkspaceMapping,
};
use ov_core::provenance::parse_frontmatter;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const CLAUDE_PRIVATE_URI: &str = "viking://agent/claude-code/memories/private.md";

#[derive(Default)]
struct MockState {
    writes: usize,
    fail_snapshot: bool,
    fail_post_snapshot: bool,
    fail_content_read: bool,
    fail_read_uri: Option<(&'static str, &'static str)>,
    fail_write_uri: Option<&'static str>,
    fail_move_from: Option<&'static str>,
    fail_session_commit: bool,
    fail_append_after_write: bool,
    memories_status: Option<&'static str>,
    session_messages: Vec<String>,
    session_peers: Vec<Option<String>>,
    session_configs: Vec<Value>,
    session_commits: usize,
    scoped_searches: Vec<Value>,
    stalled_scope_prefix: Option<&'static str>,
    long_overview: bool,
    merge_score: Option<f64>,
    merge_reflector_response: Option<&'static str>,
    files: HashMap<String, String>,
    snapshots: Vec<Value>,
    snapshot_files: HashMap<String, HashMap<String, String>>,
    restored: Vec<Value>,
    compiled: Vec<String>,
}

async fn start_mock_openviking_server(state: Arc<Mutex<MockState>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let st = state.clone();
            tokio::spawn(async move {
                let mut socket = BufReader::new(socket);
                let mut first_line = String::new();
                if socket.read_line(&mut first_line).await.unwrap() == 0 {
                    return;
                }
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap();
                let full_path = parts.next().unwrap();
                let (path, query) = full_path.split_once('?').unwrap_or((full_path, ""));
                let mut content_length = 0;
                loop {
                    let mut header = String::new();
                    if socket.read_line(&mut header).await.unwrap() == 0 {
                        return;
                    }
                    if header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            content_length = value.trim().parse().unwrap();
                        }
                    }
                }
                let mut body = vec![0; content_length];
                socket.read_exact(&mut body).await.unwrap();
                let body_json: Value = if body.is_empty() {
                    json!({})
                } else {
                    serde_json::from_slice(&body).unwrap()
                };

                let uri_param = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("uri="))
                    .map(|v| {
                        v.replace("%3A", ":")
                            .replace("%2F", "/")
                            .replace("%7E", "~")
                    })
                    .unwrap_or_default();

                let mut status = "200 OK";
                let resp_json: Value = match (method, path) {
                    ("GET", "/health") => json!({ "status": "ok", "result": { "healthy": true } }),
                    ("GET", "/api/v1/system/status") => {
                        json!({ "status": "ok", "result": { "version": "1.0.0-e2e" } })
                    }
                    ("GET", "/api/v1/observer/system") => {
                        json!({ "status": "ok", "result": { "queue": "idle", "vikingdb": "ready" } })
                    }
                    ("GET", "/api/v1/fs/ls") if uri_param == "viking://resources/projects/" => {
                        json!({ "status": "ok", "result": [
                            { "uri": "viking://resources/projects/demo/", "isDir": true },
                            { "uri": "viking://resources/projects/readme.md", "isDir": false }
                        ] })
                    }
                    ("GET", "/api/v1/fs/ls") if uri_param == "viking://~/resources/projects/" => {
                        json!({ "status": "ok", "result": [
                            { "uri": "viking://~/resources/projects/demo", "isDir": true },
                            { "uri": "viking://~/resources/projects/readme.md", "isDir": false },
                            { "uri": "viking://resources/projects/demo/", "isDir": true }
                        ] })
                    }
                    ("GET", "/api/v1/fs/ls") => {
                        let guard = st.lock().unwrap();
                        let items: Vec<Value> = guard
                            .files
                            .keys()
                            .filter(|k| k.starts_with(&uri_param))
                            .map(|k| json!({ "uri": k, "isDir": false }))
                            .collect();
                        json!({ "status": "ok", "result": items })
                    }
                    ("GET", "/api/v1/fs/tree") => {
                        let guard = st.lock().unwrap();
                        if uri_param == "viking://user/memories/" {
                            status = "403 Forbidden";
                            json!({ "status": "error", "message": "Legacy alias denied" })
                        } else if let Some(failure) = guard.memories_status {
                            status = failure;
                            let code = if failure.starts_with("503") { "UNAVAILABLE" } else { "NOT_FOUND" };
                            json!({ "status": "error", "error": { "code": code, "message": "Memories unavailable" } })
                        } else {
                            let items: Vec<Value> = guard
                                .files
                                .keys()
                                .filter(|k| k.starts_with(&uri_param))
                                .map(|k| json!({ "uri": k, "isDir": false }))
                                .collect();
                            json!({ "status": "ok", "result": items })
                        }
                    }
                    ("GET", "/api/v1/content/read") => {
                        let guard = st.lock().unwrap();
                        if let Some((_, failure)) = guard.fail_read_uri.filter(|(uri, _)| *uri == uri_param.as_str()) {
                            status = failure;
                            json!({ "status": "error", "message": "Content read failed" })
                        } else if guard.fail_content_read {
                            status = "503 Service Unavailable";
                            json!({ "status": "error", "message": "Content temporarily unavailable" })
                        } else {
                            let content = guard.files.get(&uri_param).cloned().unwrap_or_default();
                            json!({ "status": "ok", "result": { "content": content } })
                        }
                    }
                    ("GET", "/api/v1/content/overview") => {
                        let suffix = if st.lock().unwrap().long_overview { " detail".repeat(2000) } else { String::new() };
                        json!({ "status": "ok", "result": { "content": format!("L1_OVERVIEW_FOR:{uri_param}{suffix}") } })
                    }
                    ("GET", "/api/v1/content/abstract") => {
                        json!({ "status": "ok", "result": { "content": format!("L0_ABSTRACT_FOR:{}", uri_param) } })
                    }
                    ("POST", "/api/v1/content/write") => {
                        let uri = body_json["uri"].as_str().unwrap_or("").to_string();
                        let content = body_json["content"].as_str().unwrap_or("").to_string();
                        let mut guard = st.lock().unwrap();
                        if guard.fail_write_uri == Some(uri.as_str()) {
                            status = "503 Service Unavailable";
                            json!({ "status": "error", "message": "Content write failed" })
                        } else {
                            guard.writes += 1;
                            guard.files.insert(uri.clone(), content);
                            json!({ "status": "ok", "result": { "uri": uri, "written": true } })
                        }
                    }
                    ("POST", "/api/v1/fs/mv") => {
                        let from = body_json["from_uri"].as_str().unwrap_or("");
                        let to = body_json["to_uri"].as_str().unwrap_or("").to_string();
                        let mut guard = st.lock().unwrap();
                        if guard.fail_move_from == Some(from) {
                            status = "503 Service Unavailable";
                            json!({ "status": "error", "message": "Move failed" })
                        } else if let Some(v) = guard.files.remove(from) {
                            guard.writes += 1;
                            guard.files.insert(to.clone(), v);
                            json!({ "status": "ok", "result": { "moved_to": to } })
                        } else {
                            status = "404 Not Found";
                            json!({ "status": "error", "message": "Move source not found" })
                        }
                    }
                    ("DELETE", "/api/v1/fs") => {
                        let mut guard = st.lock().unwrap();
                        let prefix = format!("{}/", uri_param.trim_end_matches('/'));
                        let recursive = query.split('&').any(|part| part == "recursive=true");
                        guard.files.retain(|uri, _| {
                            uri != &uri_param && !(recursive && uri.starts_with(&prefix))
                        });
                        json!({ "status": "ok", "result": { "deleted": uri_param } })
                    }
                    ("POST", "/api/v1/search/search")
                        if body_json["mode"] == "context"
                            && body_json.get("target_uri").is_some() =>
                    {
                        status = "400 Bad Request";
                        json!({ "status": "error", "message": "Context search cannot be scoped" })
                    }
                    ("POST", "/api/v1/search/search") if body_json["mode"] == "context" => {
                        json!({
                            "status": "ok",
                            "result": {
                                "memories": [
                                    { "uri": "viking://~/memories/preferences/git.md", "score": 0.92, "snippet": "Use Conventional Commits" },
                                    { "uri": "viking://~/memories/agents/pi/notes.md", "score": 0.99, "snippet": "Private search result" }
                                ]
                            }
                        })
                    }
                    ("POST", "/api/v1/search/search") => {
                        status = "400 Bad Request";
                        json!({ "status": "error", "message": "Expected context search mode" })
                    }
                    ("POST", "/api/v1/search/find") => {
                        let (stalled, score) = {
                            let mut guard = st.lock().unwrap();
                            guard.scoped_searches.push(body_json.clone());
                            let uri = body_json["target_uri"].as_str().unwrap_or_default();
                            (guard.stalled_scope_prefix.is_some_and(|prefix| uri.starts_with(prefix)),
                             guard.merge_score.unwrap_or(0.89))
                        };
                        if stalled {
                            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                        }
                        if body_json["target_uri"] == "viking://resources/projects/demo/" {
                            json!({
                                "status": "ok",
                                "result": { "entries": [
                                    { "uri": "viking://resources/projects/demo/README.md", "score": 0.91 }
                                ] }
                            })
                        } else {
                            json!({
                                "status": "ok",
                                "result": { "memories": [
                                    { "uri": "viking://~/memories/preferences/git-old.md", "score": score }
                                ] }
                            })
                        }
                    }
                    ("POST", "/v1/chat/completions") => {
                        let prompt = body_json["messages"][1]["content"].as_str().unwrap();
                        let content = if prompt.contains("[Existing Project Directories]") {
                            assert!(prompt.contains("- viking://~/resources/projects/demo/\n"));
                            assert!(prompt.contains("fallback: viking://~/resources/projects/general/"));
                            assert!(!prompt.contains("viking://~/resources/projects/readme.md"));
                            assert!(!prompt.contains("viking://resources/projects/demo/"));
                            let destination_dir = if prompt.contains("# Misroute") {
                                "viking://resources/projects/demo/"
                            } else {
                                " viking://~/resources/projects/demo// "
                            };
                            json!({
                                "destination_dir": destination_dir,
                                "rationale": "Matches demo"
                            }).to_string()
                        } else {
                            assert!(prompt.contains("[Primary URI:"));
                            st.lock().unwrap().merge_reflector_response.unwrap_or("# Reflected memory").to_string()
                        };
                        json!({ "choices": [{ "message": { "content": content } }] })
                    }
                    ("POST", "/api/v1/sessions") => {
                        st.lock().unwrap().session_configs.push(body_json.clone());
                        json!({ "status": "ok", "result": { "session_id": "e2e-session" } })
                    }
                    ("POST", "/api/v1/sessions/e2e-session/messages") => {
                        assert!(matches!(body_json["role"].as_str(), Some("assistant" | "user")));
                        let mut guard = st.lock().unwrap();
                        let content = body_json["content"].as_str().unwrap();
                        if content == "permanently rejected" {
                            status = "422 Unprocessable Entity";
                            json!({"status":"error","message":"invalid message"})
                        } else {
                            guard.session_messages.push(content.to_string());
                            guard.session_peers.push(body_json["peer_id"].as_str().map(str::to_owned));
                            if guard.fail_append_after_write {
                                status = "500 Internal Server Error";
                                json!({"status":"error","message":"append response failed"})
                            } else {
                                json!({ "status": "ok", "result": {} })
                            }
                        }
                    }
                    ("GET", "/api/v1/sessions/e2e-session") => {
                        let guard = st.lock().unwrap();
                        json!({"status":"ok","result":{"session_id":"e2e-session","total_message_count":guard.session_messages.len()}})
                    }
                    ("POST", "/api/v1/sessions/e2e-session/commit") => {
                        let mut guard = st.lock().unwrap();
                        if guard.fail_session_commit {
                            status = "500 Internal Server Error";
                            json!({ "status": "error", "message": "Session commit failed" })
                        } else {
                            guard.session_commits += 1;
                            json!({ "status": "ok", "result": {"status":"accepted","task_id":"capture-task"} })
                        }
                    }
                    ("GET", "/api/v1/tasks/capture-task") => json!({"status":"ok","result":{"status":"completed"}}),
                    ("POST", "/api/v1/snapshot/commit") => {
                        let mut guard = st.lock().unwrap();
                        if guard.fail_snapshot {
                            status = "500 Internal Server Error";
                            json!({ "status": "error", "message": "Snapshot failed" })
                        } else {
                            let snap = format!("snap-e2e-{}", guard.snapshots.len() + 1);
                            guard.snapshots.push(body_json.clone());
                            if guard.fail_post_snapshot && body_json["message"].as_str().unwrap().starts_with("dreaming-applied-") {
                                status = "500 Internal Server Error";
                                json!({ "status": "error", "message": "Post-apply snapshot failed" })
                            } else {
                                let files = guard.files.clone();
                                guard.snapshot_files.insert(snap.clone(), files);
                                json!({ "status": "ok", "result": { "commit_oid": snap } })
                            }
                        }
                    }
                    ("POST", "/api/v1/snapshot/restore") => {
                        if body_json.get("snapshot_id").is_some() {
                            status = "400 Bad Request";
                            json!({ "status": "error", "message": "snapshot_id is unsupported" })
                        } else if !body_json["source_commit"].as_str().is_some_and(|s| !s.is_empty()) {
                            status = "400 Bad Request";
                            json!({ "status": "error", "message": "source_commit is required" })
                        } else if !body_json["project_dir"].as_str().is_some_and(|s| s.starts_with("viking://") && s != "viking://") {
                            status = "403 Forbidden";
                            json!({ "status": "error", "message": "project_dir is required for User API Keys" })
                        } else {
                            let mut guard = st.lock().unwrap();
                            guard.restored.push(body_json.clone());
                            let scope = body_json["project_dir"].as_str().unwrap().trim_end_matches('/');
                            let prefix = format!("{scope}/");
                            let in_scope = |uri: &str| uri == scope || uri.starts_with(&prefix);
                            let snapshot = &guard.snapshot_files[body_json["source_commit"].as_str().unwrap()];
                            let files: Vec<_> = snapshot.iter().filter(|(uri, _)| in_scope(uri))
                                .map(|(uri, content)| (uri.clone(), content.clone())).collect();
                            if files.is_empty() {
                                status = "404 Not Found";
                                json!({ "status": "error", "message": "Directory missing from snapshot" })
                            } else {
                                guard.files.retain(|uri, _| !in_scope(uri));
                                guard.files.extend(files);
                                json!({ "status": "ok", "result": { "restored": body_json["source_commit"] } })
                            }
                        }
                    }
                    ("POST", "/api/v1/compile") => {
                        assert_eq!(body_json["skill"], "wiki");
                        assert_eq!(body_json["from"], json!(["viking://resources/projects/demo/"]));
                        let to = body_json["to"].as_str().unwrap().to_string();
                        let mut guard = st.lock().unwrap();
                        guard.writes += 1;
                        guard.files.insert(format!("{to}HOME.md"), "# Generated Wiki".into());
                        guard.compiled.push(to);
                        json!({ "status": "ok", "result": { "task_id": "task-e2e-1" } })
                    }
                    _ => {
                        status = "404 Not Found";
                        json!({ "status": "error", "message": "Unknown endpoint" })
                    }
                };

                let body_bytes = serde_json::to_vec(&resp_json).unwrap();
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body_bytes.len()
                );
                let _ = socket.get_mut().write_all(header.as_bytes()).await;
                let _ = socket.get_mut().write_all(&body_bytes).await;
            });
        }
    });

    format!("http://{}", addr)
}

fn test_client(url: &str, account: &str) -> OvClient {
    OvClient::new(&OvCliConfig {
        url: url.to_string(),
        account: Some(account.to_string()),
        api_key: "test-key".to_string(),
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn e2e_mcp_stdio_handshake_ping_and_parse_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ov-bridge"))
        .args(["mcp", "--agent", "claude-code"])
        .env("OPENVIKING_DISABLE_HOOKS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })).unwrap();
    writeln!(stdin, "malformed JSON").unwrap();
    writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" })).unwrap();
    writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" })).unwrap();
    writeln!(stdin, "{}", json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "recall", "arguments": { "query": "reflector" } }
    })).unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(responses.len(), 5);
    assert!(responses[0]["result"]["protocolVersion"].is_string());
    assert_eq!(responses[1], json!({
        "jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}
    }));
    assert_eq!(responses[2], json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
    assert!(responses[3]["result"]["tools"].as_array().unwrap()
        .iter().any(|tool| tool["name"] == "recall"));
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_eq!(responses[4]["result"]["content"][0]["text"], "OpenViking tools are disabled for this reflector process");
}

#[test]
fn e2e_reflector_hooks_skip_input_and_emit_host_output() {
    for (event, agent, expected) in [
        ("stop", "codex", "{}"),
        ("pre-invocation", "antigravity", "{}"),
        ("session-start", "claude-code", ""),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ov-bridge"))
            .args(["hook", event, "--agent", agent])
            .env("OPENVIKING_DISABLE_HOOKS", "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
    }
}

#[tokio::test]
async fn e2e_mcp_recall_and_capture_are_profile_scoped() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let other = test_client(&url, "personal");
    let db = DesktopDb::open_in_memory().unwrap();
    let mut policy = PolicyConfig::default();
    policy.routing.auto_map_git_repo = false;
    policy.routing.respect_project_policy_file = false;
    policy.routing.inbox_uri.clear();
    let agent = AgentId::ClaudeCode;
    let unscoped = json!({ "name": "recall", "arguments": { "query": "git commit style" } });
    let text = mcp::call_tool(agent, &unscoped, &policy, &client, Some(&db)).await.unwrap();
    assert!(text.contains("<openviking_memory_reference") && text.contains("readonly=\"true\""));
    assert!(text.contains("Auto-Expanded L1 Overview"));
    assert!(text.contains("L1_OVERVIEW_FOR:viking://~/memories/preferences/git.md"));
    assert!(!text.contains("Private search result"));
    assert!(!text.contains("viking://~/memories/agents/pi/notes.md"));
    assert!(state.lock().unwrap().scoped_searches.is_empty());

    let scoped = json!({ "name": "recall", "arguments": {
        "query": "Read project notes", "target_uri": "viking://resources/projects/demo/", "limit": 2
    } });
    let mut tight_policy = policy.clone();
    tight_policy.budget.agent_overrides.clear();
    tight_policy.budget.max_recall_tokens = 8;
    let text = mcp::call_tool(agent, &scoped, &tight_policy, &other, Some(&db)).await.unwrap();
    assert!(!text.contains("Auto-Expanded L1 Overview"));
    assert_eq!(state.lock().unwrap().scoped_searches.as_slice(), &[json!({
        "query": "Read project notes", "target_uri": "viking://resources/projects/demo/", "limit": 2
    })]);

    let work_events = db.list_events_for_profile(&client.profile_key(), None, 10).unwrap();
    assert_eq!(work_events.len(), 1);
    assert_eq!(work_events[0].layer.as_deref(), Some("l1"));
    assert_eq!(work_events[0].target_uri.as_deref(), Some("viking://~/memories/preferences/git.md"));
    let other_events = db.list_events_for_profile(&other.profile_key(), None, 10).unwrap();
    assert_eq!(other_events.len(), 1);
    assert_eq!(other_events[0].layer.as_deref(), Some("l0"));
    assert_eq!(other_events[0].target_uri, None);
    assert!(db.hit_stats_by_uri_for_profile(&other.profile_key()).unwrap().is_empty());
    assert!(db.list_events_for_profile("", None, 10).unwrap().is_empty());

    policy.routing.workspace_mappings.push(WorkspaceMapping {
        local_path: std::env::current_dir().unwrap(),
        resource_uri: "viking://resources/projects/demo".into(),
        visibility: VisibilityScope::Shared,
    });
    let text = mcp::call_tool(agent, &unscoped, &policy, &client, None).await.unwrap();
    assert!(text.contains("viking://resources/projects/demo/README.md"));
    let global = json!({ "name": "recall", "arguments": {
        "query": "git commit style", "target_uri": ""
    } });
    let text = mcp::call_tool(agent, &global, &policy, &client, None).await.unwrap();
    assert!(text.contains("viking://~/memories/preferences/git.md"));
    assert!(state.lock().unwrap().scoped_searches.iter().any(|req| req["target_uri"] ==
        "viking://resources/projects/demo/"));
    assert!(state.lock().unwrap().scoped_searches.iter().any(|req|
        req["target_uri"].as_str().is_some_and(|uri|uri.starts_with("viking://~/peers/project-"))));
    assert_eq!(state.lock().unwrap().scoped_searches.len(), 3);
    state.lock().unwrap().long_overview = true;
    let limited = json!({ "name": "recall", "arguments": { "query": "git commit style", "limit": 1 } });
    let text = mcp::call_tool(agent, &limited, &policy, &client, None).await.unwrap();
    assert!(text.contains("- [general] README.md"));
    assert!(text.contains("L1_OVERVIEW_FOR:viking://resources/projects/demo/README.md"));
    assert!(!text.contains("git-old.md"));
    policy.routing.workspace_mappings.clear();

    let protected = json!({ "name": "remember", "arguments": {
        "uri": "viking://resources/projects/demo/CLAUDE.md/", "content": "protected"
    } });
    let failure = mcp::call_tool(agent, &protected, &policy, &client, Some(&db)).await.unwrap_err();
    assert!(failure.contains("Write denied"));
    assert_eq!(state.lock().unwrap().writes, 0);

    policy.routing.inbox_uri = "viking://~/resources/inbox///".to_string();
    let note = json!({ "name": "remember", "arguments": { "content": "Prefer Rust + Tauri" } });
    mcp::call_tool(agent, &note, &policy, &client, Some(&db)).await.unwrap();
    let guard = state.lock().unwrap();
    let (uri, body) = guard.files.iter().next().unwrap();
    let filename = uri.strip_prefix("viking://~/resources/inbox/note-").unwrap();
    let (_, suffix) = filename.strip_suffix(".md").unwrap().rsplit_once('-').unwrap();
    assert_eq!(suffix.len(), 4);
    assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let (provenance, body) = parse_frontmatter(body);
    assert_eq!(provenance.unwrap().source_agent, "claude-code");
    assert_eq!(body, "Prefer Rust + Tauri");
    drop(guard);
    assert_eq!(db.count_sessions_since(&client.profile_key(), 0).unwrap(), 1);
    assert_eq!(db.count_sessions_since(&other.profile_key(), 0).unwrap(), 0);

    let explicit = json!({ "name": "remember", "arguments": {
        "uri": "viking://~/memories/preferences/editor.md",
        "content": "Prefer Rust + Tauri",
        "verified": true
    } });
    mcp::call_tool(agent, &explicit, &policy, &client, None).await.unwrap();
    let private = json!({ "name": "remember", "arguments": {
        "visibility": "private", "content": "Agent private scratchpad"
    } });
    mcp::call_tool(agent, &private, &policy, &client, None).await.unwrap();
    let guard = state.lock().unwrap();
    let saved_editor = &guard.files["viking://~/memories/preferences/editor.md"];
    let (provenance, body) = parse_frontmatter(saved_editor);
    let provenance = provenance.unwrap();
    assert_eq!(provenance.source_agent, "claude-code");
    assert!(provenance.verified);
    assert_eq!(body, "Prefer Rust + Tauri");
    let (_, saved_private) = guard.files.iter().find(|(uri, _)| {
        uri.starts_with("viking://agent/claude-code/memories/note-")
    }).expect("Private note must use the agent namespace");
    let (provenance, body) = parse_frontmatter(saved_private);
    assert_eq!(provenance.unwrap().scope, "agent_private");
    assert_eq!(body, "Agent private scratchpad");
}

#[tokio::test]
async fn e2e_mcp_denies_foreign_agent_private_uris() {
    let state = Arc::new(Mutex::new(MockState {
        files: HashMap::from([(CLAUDE_PRIVATE_URI.into(), "Claude private content".into())]),
        ..Default::default()
    }));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let policy = PolicyConfig::default();
    let read = json!({ "name": "read_uri", "arguments": { "uri": CLAUDE_PRIVATE_URI, "layer": "l2" } });
    for call in [
        read.clone(),
        json!({ "name": "remember", "arguments": { "uri": CLAUDE_PRIVATE_URI, "content": "Overwrite" } }),
        json!({ "name": "list_uri", "arguments": { "uri": "viking://~/resources/agents/claude-code/" } }),
        json!({ "name": "recall", "arguments": { "target_uri": "viking://user/alice/memories/agents/claude-code/", "query": "private" } }),
    ] {
        let result = mcp::call_tool(AgentId::Codex, &call, &policy, &client, None).await;
        assert_eq!(result.unwrap_err(), "Access denied: URI belongs to another agent's private namespace");
    }
    assert_eq!(state.lock().unwrap().writes, 0);
    assert!(state.lock().unwrap().scoped_searches.is_empty());
    let result = mcp::call_tool(AgentId::ClaudeCode, &read, &policy, &client, None).await.unwrap();
    assert!(result.contains("Claude private content"));
}

#[tokio::test]
async fn e2e_hook_outputs_and_stop_capture() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let mut policy = PolicyConfig::default();
    let tmp = tempdir().unwrap();
    std::fs::write(
        tmp.path().join("RESOURCE_POLICY.md"),
        "resource_uri: viking://resources/projects/demo\n",
    ).unwrap();

    let prompt = json!({ "workspacePaths": [tmp.path()], "lastUserInput": "Read project notes" });
    let agent = AgentId::Antigravity;
    let text = hook::execute_hook("pre-invocation", agent, &prompt, &policy, &client, Some(&db)).await;
    assert!(text.contains("viking://resources/projects/demo/README.md"));
    let output = hook::hook_stdout("pre-invocation", agent, &text).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), json!({
        "injectSteps": [{ "systemMessage": { "systemMessage": text } }]
    }));
    {
        let guard = state.lock().unwrap();
        assert_eq!(guard.scoped_searches.len(),2);
        assert!(guard.scoped_searches.contains(&json!({
            "query": "Read project notes", "target_uri": "viking://resources/projects/demo/", "limit": 5
        })));
        assert!(guard.scoped_searches.iter().any(|req| req["target_uri"].as_str().is_some_and(|uri| uri.starts_with("viking://~/peers/project-"))));
    }

    let agent = AgentId::Codex;
    let input = json!({ "cwd": tmp.path(), "message": "Read project notes" });
    for (event, hook_event_name, expected) in [
        ("session-start", "SessionStart", "L0_ABSTRACT_FOR:viking://resources/projects/demo/"),
        ("user-prompt-submit", "UserPromptSubmit", "viking://resources/projects/demo/README.md"),
    ] {
        let text = hook::execute_hook(event, agent, &input, &policy, &client, Some(&db)).await;
        assert!(text.contains(expected), "Missing {expected} in {text}");
        assert!(text.contains("readonly=\"true\""));
        if event == "session-start" {
            assert!(text.contains("viking://agent/codex/memories/"));
            assert!(text.contains("L0_ABSTRACT_FOR:viking://~/memories/"));
        }
        assert_eq!(hook::hook_stdout(event, AgentId::Pi, &text).as_deref(), Some(text.as_str()));
        for hook_agent in [AgentId::Codex, AgentId::ClaudeCode] {
            let output = hook::hook_stdout(event, hook_agent, &text).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&output).unwrap(), json!({
                "hookSpecificOutput": { "hookEventName": hook_event_name, "additionalContext": text }
            }));
        }
    }
    assert_eq!(hook::hook_stdout("pre-invocation", AgentId::Antigravity, "").as_deref(), Some("{}"));
    assert_eq!(hook::hook_stdout("stop", AgentId::Pi, ""), None);

    let transcript = tmp.path().join("transcript.jsonl");
    std::fs::write(&transcript, [
        json!({ "type": "response_item", "payload": { "role": "assistant", "content": [
            { "type": "output_text", "text": "Remember this decision" },
            { "type": "output_text", "text": "Second line" },
            { "type": "tool_use", "text": "Exclude this" }
        ] } }).to_string(),
        json!({ "type": "assistant", "message": { "role": "assistant", "content": [
            { "type": "tool_use", "text": "Exclude this too" }
        ] } }).to_string(),
        json!({ "role": "user", "content": "Ignore user text" }).to_string(),
        "{".to_string(),
    ].join("\n")).unwrap();
    let private_transcript = tmp.path().join("private.jsonl");
    std::fs::write(&private_transcript, json!({ "uuid": "priv-1", "role": "user", "content": "Private secret" }).to_string()).unwrap();
    let private_input = json!({ "cwd": tmp.path(), "transcriptPath": private_transcript });
    policy.global.capture_enabled = false;
    hook::execute_hook("stop", agent, &private_input, &policy, &client, Some(&db)).await;
    assert!(state.lock().unwrap().session_messages.is_empty());
    assert_eq!(db.count_sessions_since(&client.profile_key(), 0).unwrap(), 0);

    policy.global.capture_enabled = true;
    // Re-running on private_input after resuming capture must NOT backfill the private message
    hook::execute_hook("stop", agent, &private_input, &policy, &client, Some(&db)).await;
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().pending, 0);

    let input = json!({ "cwd": tmp.path(), "transcriptPath": transcript });
    hook::execute_hook("stop", agent, &json!({}), &policy, &client, Some(&db)).await;
    hook::execute_hook("stop", agent, &input, &policy, &client, Some(&db)).await;
    assert!(state.lock().unwrap().session_messages.is_empty());
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().pending, 2);
    let text = hook::execute_hook("stop", agent, &input, &policy, &client, Some(&db)).await;
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().pending, 2);
    assert_eq!(ov_core::capture::sync_capture(&db, &client, 0, false).await.unwrap(), 1);
    assert_eq!(ov_core::capture::sync_capture(&db, &client, 0, false).await.unwrap(), 0);
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().extraction_tasks,
        vec![("capture-task".to_string(),"accepted".to_string())]);
    ov_core::capture::refresh_extraction_tasks(&db,&client).await.unwrap();
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().extraction_tasks[0].1,"completed");
    assert_eq!(hook::hook_stdout("stop", agent, &text).as_deref(), Some("{}"));
    assert_eq!(hook::hook_stdout("stop", AgentId::Antigravity, &text).as_deref(), Some("{}"));
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().pending, 0);
    assert_eq!(db.count_sessions_since(&client.profile_key(), 0).unwrap(), 1);
    assert!(db.list_events_for_profile("", None, 10).unwrap().is_empty());
    let other = test_client(&url, "personal");
    assert!(db.list_events_for_profile(&other.profile_key(), None, 10).unwrap().is_empty());
    let guard = state.lock().unwrap();
    assert_eq!(guard.session_commits, 1);
    assert_eq!(guard.session_messages, vec!["Remember this decision\nSecond line".to_string(), "Ignore user text".to_string()]);
    assert_eq!(guard.session_configs[0]["memory_policy"]["peer"]["enabled"], true);
    assert!(guard.session_peers[0].as_deref().unwrap().starts_with("project-"));
    assert_eq!(guard.session_peers[0],guard.session_peers[1]);
}

#[tokio::test]
async fn e2e_dreaming_snapshot_failure_isolation_and_rollback() {
    const CONCURRENT_INBOX: &str = "viking://~/resources/inbox/concurrent.md";
    const INBOX_NOTE: &str = "viking://~/resources/inbox/scratch.md";
    const ROUTED_NOTE: &str = "viking://~/resources/projects/general/scratch.md";
    const PROJECT_SIBLING: &str = "viking://~/resources/projects/general/new.md";
    const MEMORY_SIBLING: &str = "viking://~/memories/preferences/new.md";
    const WIKI_OUTPUT: &str = "viking://resources/desktop-dreaming-wiki/HOME.md";
    let state = Arc::new(Mutex::new(MockState {
        files: HashMap::from([
            (INBOX_NOTE.into(), "# Scratch note".into()),
            (CONCURRENT_INBOX.into(), "# Note to edit during review".into()),
            (
                "viking://~/memories/preferences/git.md".to_string(),
                "# Git Commit Style\nUse English Conventional Commits for clarity.".to_string(),
            ),
            (
                "viking://~/memories/preferences/git-old.md".to_string(),
                "# Git Commit Style\nUse English Conventional Commits for clarity.\nLegacy line.".to_string(),
            ),
        ]),
        ..Default::default()
    }));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let other = test_client(&url, "personal");
    let db = DesktopDb::open_in_memory().unwrap();
    let policy = PolicyConfig::default();

    let (run, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    assert_eq!(run.status, "ready_for_review");
    assert_eq!(run.profile_key, client.profile_key());
    let inbox_move = staged.iter().find(|c| c.target_uri == INBOX_NOTE).unwrap();
    let conflict = staged.iter().find(|c| c.target_uri == CONCURRENT_INBOX).unwrap();
    let merge = staged.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    let evidence: Value = serde_json::from_str(&merge.evidence_json).unwrap();
    assert_eq!(evidence["reflector"], "deterministic");
    let (provenance, _) = parse_frontmatter(merge.after_content.as_deref().unwrap());
    let provenance = provenance.unwrap();
    assert_eq!(provenance.updated_by, "dreaming:deterministic");
    assert!(!provenance.verified);
    assert_eq!(provenance.evidence_uris.len(), 2);
    assert!(staged.iter().any(|c| c.action == ChangeAction::WikiCompile));

    db.update_change_decision_for_profile(&client.profile_key(), &inbox_move.id, ReviewStatus::Accepted, None).unwrap();
    let mut retry_policy = policy.clone();
    retry_policy.lifecycle.dreaming.auto_snapshot_before_run = false;
    state.lock().unwrap().fail_content_read = true;
    let report = apply_accepted_changes(&client, &db, &retry_policy).await.unwrap();
    assert!(report.conflict_ids.is_empty() && report.applied_ids.is_empty());
    assert_eq!(report.failed_items.len(), 1);
    assert_eq!(report.failed_items[0].0, inbox_move.id);
    assert!(report.failed_items[0].1.contains("503"));
    let accepted = db.list_pending_changes_for_profile(&client.profile_key(), Some("accepted")).unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, inbox_move.id);
    state.lock().unwrap().fail_content_read = false;

    for change in &staged {
        db.update_change_decision_for_profile(&client.profile_key(), &change.id, ReviewStatus::Accepted, None).unwrap();
    }

    assert!(db.list_runs_for_profile(&other.profile_key(), 10).unwrap().is_empty());
    assert!(db.list_pending_changes_for_profile(&other.profile_key(), None).unwrap().is_empty());
    let other_report = apply_accepted_changes(&other, &db, &policy).await.unwrap();
    assert!(other_report.applied_ids.is_empty() && other_report.snapshot_id.is_none());

    state.lock().unwrap().fail_snapshot = true;
    let error = apply_accepted_changes(&client, &db, &policy).await.unwrap_err();
    assert!(error.contains("Snapshot failed"));
    {
        let mut guard = state.lock().unwrap();
        assert_eq!(guard.writes, 0);
        assert!(guard.snapshots.is_empty());
        guard.fail_snapshot = false;
        guard.fail_post_snapshot = true;
        guard.files.insert(CONCURRENT_INBOX.into(), "# Concurrent edit".into());
    }
    assert_eq!(
        db.list_pending_changes_for_profile(&client.profile_key(), Some("accepted")).unwrap().len(),
        staged.len(),
    );

    let report = apply_accepted_changes(&client, &db, &policy).await.unwrap();
    assert_eq!(report.snapshot_id.as_deref(), Some("snap-e2e-1"));
    assert_eq!(report.conflict_ids, [conflict.id.clone()]);
    assert!(report.applied_ids.contains(&inbox_move.id));
    assert!(report.applied_ids.contains(&merge.id));
    assert_eq!(report.failed_items.len(), 1);
    assert_eq!(report.failed_items[0].0, "post-apply-snapshot");
    assert!(report.failed_items[0].1.contains("500"));
    {
        let mut guard = state.lock().unwrap();
        assert_eq!(guard.compiled.as_slice(), &["viking://resources/desktop-dreaming-wiki/"]);
        assert_eq!(guard.snapshots.len(), 2);
        assert!(guard.snapshots[0]["message"].as_str().unwrap().starts_with("dreaming-commit-"));
        assert!(guard.snapshots[1]["message"].as_str().unwrap().starts_with("dreaming-applied-"));
        assert_eq!(guard.snapshots[0]["paths"], json!([
            "viking://resources/desktop-dreaming-wiki/",
            "viking://~/archive/merged-git-old.md",
            "viking://~/memories/preferences/git-old.md",
            "viking://~/memories/preferences/git.md",
            CONCURRENT_INBOX,
            INBOX_NOTE,
            "viking://~/resources/projects/general/concurrent.md",
            ROUTED_NOTE,
        ]));
        assert_eq!(guard.snapshots[1]["paths"], guard.snapshots[0]["paths"]);
        assert_eq!(guard.files[ROUTED_NOTE], "# Scratch note");
        assert_eq!(guard.files[&merge.target_uri], merge.after_content.as_deref().unwrap().replacen("verified: false", "verified: true", 1));
        assert!(guard.files.contains_key(WIKI_OUTPUT));
        guard.files.insert(PROJECT_SIBLING.into(), "# New project note".into());
        guard.files.insert(MEMORY_SIBLING.into(), "# New memory".into());
    }
    assert!(rollback_dreaming_run(&other, &db, &run.id).await.is_err());
    assert!(state.lock().unwrap().restored.is_empty());
    rollback_dreaming_run(&client, &db, &run.id).await.unwrap();
    let guard = state.lock().unwrap();
    assert_eq!(guard.restored.as_slice(), &[
        json!({ "source_commit": "snap-e2e-1", "project_dir": "viking://resources/desktop-dreaming-wiki" }),
    ]);
    assert_eq!(guard.files[INBOX_NOTE], "# Scratch note");
    assert_eq!(guard.files[CONCURRENT_INBOX], "# Concurrent edit");
    assert_eq!(guard.files[&merge.target_uri], merge.before_content.as_deref().unwrap());
    assert_eq!(guard.files[PROJECT_SIBLING], "# New project note");
    assert_eq!(guard.files[MEMORY_SIBLING], "# New memory");
    assert!(!guard.files.contains_key(ROUTED_NOTE));
    assert!(!guard.files.contains_key(WIKI_OUTPUT));
    assert_eq!(db.get_run(&run.id).unwrap().unwrap().status, "rolled_back");
}

#[tokio::test]
async fn e2e_dreaming_reflector_routes_inbox_and_adds_merge_provenance() {
    let state = Arc::new(Mutex::new(MockState {
        files: HashMap::from([
            ("viking://~/resources/inbox/scratch.md".into(), "# Scratch note".into()),
            ("viking://~/resources/inbox/misroute.md".into(), "# Misroute".into()),
            (
                "viking://~/memories/preferences/git.md".into(),
                "---\nsource_agent: claude-code\ntitle: Commit Style\ntags: [git]\n---\n# Git Commit Style\nUse English Conventional Commits.".into(),
            ),
            (
                "viking://~/memories/preferences/git-old.md".into(),
                "# Commit Style\nUse clear English commit messages.".into(),
            ),
        ]),
        ..Default::default()
    }));
    let url = start_mock_openviking_server(state).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let mut policy = PolicyConfig::default();
    policy.lifecycle.dreaming.reflector_mode = ReflectorMode::OpenaiCompatible;
    policy.lifecycle.dreaming.reflector_endpoint = format!("{url}/v1");
    policy.lifecycle.dreaming.reflector_model = "e2e-reflector".into();

    let (_, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let inbox = staged.iter().find(|c| c.target_uri.ends_with("/scratch.md")).unwrap();
    assert_eq!(inbox.destination_uri.as_deref(), Some("viking://~/resources/projects/demo/scratch.md"));
    let evidence: Value = serde_json::from_str(&inbox.evidence_json).unwrap();
    assert_eq!(evidence["reflector"], "llm:e2e-reflector");
    let misroute = staged.iter().find(|c| c.target_uri.ends_with("/misroute.md")).unwrap();
    assert_eq!(misroute.destination_uri.as_deref(), Some("viking://~/resources/projects/general/misroute.md"));
    let evidence: Value = serde_json::from_str(&misroute.evidence_json).unwrap();
    assert_eq!(evidence["reflector"], "deterministic");

    let merge = staged.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    let content = merge.after_content.as_deref().unwrap();
    assert!(content.contains("\ntitle: Commit Style\ntags: [git]\n"));
    let (provenance, body) = parse_frontmatter(content);
    assert_eq!(body, "# Reflected memory");
    let provenance = provenance.unwrap();
    assert_eq!(provenance.source_agent, "claude-code");
    assert_eq!(provenance.updated_by, "dreaming:llm:e2e-reflector");
    assert!(!provenance.verified);
    assert_eq!(provenance.evidence_uris.len(), 2);
}


#[tokio::test]
async fn e2e_failed_commit_can_retry_without_replaying_messages() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let (_, messages) = ov_core::capture::collect(&json!({"messages":[{"role":"assistant","content":"one decision"}]}));
    db.enqueue_capture(&client.profile_key(),"session","codex","viking://resources/projects/demo/",None,&messages,false).unwrap();
    state.lock().unwrap().fail_session_commit = true;
    assert!(ov_core::capture::sync_capture(&db,&client,0,false).await.is_err());
    assert_eq!(db.capture_health(&client.profile_key()).unwrap().uncertain_sessions,0);
    state.lock().unwrap().fail_session_commit = false;
    assert_eq!(ov_core::capture::sync_capture(&db,&client,0,false).await.unwrap(),1);
    assert_eq!(state.lock().unwrap().session_messages.len(),1);
}

#[tokio::test]
async fn e2e_capture_threshold_and_forced_flush() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let (_, messages) = ov_core::capture::collect(&json!({"messages":[{"role":"assistant","content":"small batch"}]}));
    let profile = client.profile_key();
    db.enqueue_capture(&profile,"session","codex","viking://resources/projects/demo/",Some("project-demo"),&messages,false).unwrap();
    assert_eq!(ov_core::capture::sync_capture(&db,&client,100,false).await.unwrap(),0);
    assert_eq!(db.capture_health(&profile).unwrap().pending,1);
    db.enqueue_capture(&profile,"session","codex","viking://resources/projects/demo/",Some("project-demo"),&[],true).unwrap();
    assert_eq!(ov_core::capture::sync_capture(&db,&client,100,false).await.unwrap(),1);
    let guard = state.lock().unwrap();
    assert_eq!(guard.session_peers,vec![Some("project-demo".to_string())]);
    assert_eq!(guard.session_commits,1);
}

#[tokio::test]
async fn e2e_manual_sync_reconciles_append_acknowledgement_loss() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let (_,messages) = ov_core::capture::collect(&json!({"messages":[{"role":"assistant","content":"decision"}]}));
    let profile = client.profile_key();
    db.enqueue_capture(&profile,"session","codex","viking://resources/projects/demo/",None,&messages,false).unwrap();
    state.lock().unwrap().fail_append_after_write = true;
    assert!(ov_core::capture::sync_capture(&db,&client,0,false).await.is_err());
    assert_eq!(db.capture_health(&profile).unwrap().uncertain_sessions,1);
    state.lock().unwrap().fail_append_after_write = false;
    assert_eq!(ov_core::capture::sync_capture(&db,&client,0,false).await.unwrap(),0);
    assert_eq!(ov_core::capture::sync_capture(&db,&client,0,true).await.unwrap(),1);
    assert_eq!(db.capture_health(&profile).unwrap().uncertain_sessions,0);
    assert_eq!(state.lock().unwrap().session_messages.len(),1);
}

#[tokio::test]
async fn e2e_short_prompts_and_pi_temporary_context_are_not_suppressed() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let policy = PolicyConfig::default();
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("RESOURCE_POLICY.md"),"resource_uri: viking://resources/projects/demo\n").unwrap();
    let input = json!({"cwd":tmp.path(),"session_id":"pi-session","prompt":"继续"});
    let first = hook::execute_hook("user-prompt-submit",AgentId::Pi,&input,&policy,&client,Some(&db)).await;
    let second = hook::execute_hook("user-prompt-submit",AgentId::Pi,&input,&policy,&client,Some(&db)).await;
    assert!(first.contains("viking://resources/projects/demo/README.md"));
    assert_eq!(first,second);
}

#[tokio::test]
async fn e2e_recall_keeps_completed_scope_and_excludes_private_turns_across_routes() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let other = test_client(&url, "personal");
    let db = DesktopDb::open_in_memory().unwrap();
    let mut policy = PolicyConfig::default();
    let tmp = tempdir().unwrap();
    let route = tmp.path().join("RESOURCE_POLICY.md");
    std::fs::write(&route, "resource_uri: viking://resources/projects/previous\n").unwrap();
    let mut input = json!({"cwd":tmp.path(),"session_id":"stable","prompt":"continue","messages":[
        {"id":"private","role":"user","content":"Private transcript turn"}
    ]});

    policy.global.capture_enabled = false;
    hook::execute_hook("stop", AgentId::Pi, &input, &policy, &client, Some(&db)).await;
    hook::execute_hook("user-prompt-submit", AgentId::Pi, &input, &policy, &client, Some(&db)).await;
    {
        let mut guard = state.lock().unwrap();
        assert!(!guard.scoped_searches.is_empty());
        assert!(guard.scoped_searches.iter().all(|request| request["query"] == "continue"));
        guard.scoped_searches.clear();
        guard.stalled_scope_prefix = Some("viking://~/peers/");
    }

    policy.global.capture_enabled = true;
    std::fs::write(&route, "resource_uri: viking://resources/projects/demo\n").unwrap();
    input["messages"].as_array_mut().unwrap().push(json!({
        "id":"public","role":"user","content":"Public transcript turn"
    }));
    hook::execute_hook("stop", AgentId::Pi, &input, &policy, &other, Some(&db)).await;
    assert_eq!(ov_core::capture::sync_capture(&db, &other, 0, false).await.unwrap(), 1);
    let text = hook::execute_hook("user-prompt-submit", AgentId::Pi, &input, &policy, &other, Some(&db)).await;
    assert!(text.contains("viking://resources/projects/demo/README.md"));
    let guard = state.lock().unwrap();
    assert_eq!(guard.session_messages, ["Public transcript turn"]);
    assert_eq!(guard.scoped_searches.len(), 2);
    assert!(guard.scoped_searches.iter().all(|request| request["query"] == "Public transcript turn\ncontinue"));
}

#[tokio::test]
async fn e2e_capture_drains_healthy_sessions_after_permanent_rejection() {
    let state = Arc::new(Mutex::new(MockState::default()));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let profile = client.profile_key();
    let db = DesktopDb::open_in_memory().unwrap();
    for (session, content) in [("bad", "permanently rejected"), ("good", "healthy")] {
        let (_, messages) = ov_core::capture::collect(&json!({
            "messages": [{"role":"user","content":content}]
        }));
        db.enqueue_capture(&profile, session, "codex", "viking://resources/projects/demo/", None, &messages, false).unwrap();
    }

    assert_eq!(ov_core::capture::sync_capture(&db, &client, 0, true).await.unwrap(), 1);
    assert_eq!(ov_core::capture::sync_capture(&db, &client, 0, true).await.unwrap(), 0);
    assert!(!db.has_capture_work(&profile).unwrap());
    assert_eq!(db.capture_health(&profile).unwrap().last_error.as_deref(), Some("Message delivery rejected (HTTP 422)"));
    assert_eq!(state.lock().unwrap().session_messages, ["healthy"]);
}

#[tokio::test]
async fn e2e_dreaming_continues_when_memories_are_absent() {
    let state = Arc::new(Mutex::new(MockState {
        files: HashMap::from([("viking://~/resources/inbox/scratch.md".into(), "# Scratch note".into())]),
        ..Default::default()
    }));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let policy = PolicyConfig::default();

    for status in ["404 Not Found", "200 OK"] {
        state.lock().unwrap().memories_status = Some(status);
        let (_, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
        let actions: Vec<_> = staged.iter().map(|change| change.action).collect();
        assert_eq!(actions, [ChangeAction::MoveFromInbox, ChangeAction::WikiCompile]);
    }
    state.lock().unwrap().memories_status = Some("503 Service Unavailable");
    let error = run_dreaming(&client, &db, &policy, "e2e").await.unwrap_err();
    assert!(error.contains("503"));
}


#[tokio::test]
async fn e2e_merge_archives_peer_upgrades_verified_and_rolls_back_both() {
    const PRIMARY: &str = "viking://~/memories/preferences/git.md";
    const PEER: &str = "viking://~/memories/preferences/git-old.md";
    const PEER_ARCHIVED: &str = "viking://~/archive/merged-git-old.md";

    let state = Arc::new(Mutex::new(MockState {
        files: HashMap::from([
            (PRIMARY.into(), "# Git Style\nUse English Conventional Commits.".into()),
            (PEER.into(), "# Git Style\nUse English Conventional Commits.\nOld note.".into()),
        ]),
        ..Default::default()
    }));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let policy = PolicyConfig::default();

    let (run, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let merge = staged.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    let (staged_prov, _) = parse_frontmatter(merge.after_content.as_deref().unwrap());
    assert!(!staged_prov.unwrap().verified, "Staged merge must remain unverified");

    db.update_change_decision_for_profile(&client.profile_key(), &merge.id, ReviewStatus::Accepted, None).unwrap();
    let report = apply_accepted_changes(&client, &db, &policy).await.unwrap();
    assert_eq!(report.applied_ids, vec![merge.id.clone()]);

    {
        let guard = state.lock().unwrap();
        let applied_primary = guard.files.get(PRIMARY).expect("primary exists");
        let (applied_prov, _) = parse_frontmatter(applied_primary);
        assert!(applied_prov.unwrap().verified, "Approved applied merge must be verified");
        assert!(!guard.files.contains_key(PEER), "peer must be removed from active memories");
        assert!(guard.files.contains_key(PEER_ARCHIVED), "peer must be archived");
    }

    {
        let mut guard = state.lock().unwrap();
        guard.fail_move_from = Some(PEER_ARCHIVED);
        guard.fail_write_uri = Some(PEER);
    }
    let error = rollback_dreaming_run(&client, &db, &run.id).await.unwrap_err();
    assert!(error.contains("Could not restore merge peer"));
    {
        let mut guard = state.lock().unwrap();
        assert!(guard.files.contains_key(PEER_ARCHIVED));
        assert!(!guard.files.contains_key(PEER));
        guard.fail_write_uri = None;
    }
    rollback_dreaming_run(&client, &db, &run.id).await.unwrap();
    {
        let guard = state.lock().unwrap();
        assert!(guard.files.contains_key(PEER), "rollback must restore peer");
        assert!(!guard.files.contains_key(PEER_ARCHIVED), "rollback must clear archived peer");
    }
}

#[tokio::test]
async fn e2e_merge_detects_peer_drift_and_compensates_archive_failure() {
    const PRIMARY: &str = "viking://~/memories/preferences/git.md";
    const PEER: &str = "viking://~/memories/preferences/git-old.md";
    let original = HashMap::from([
        (PRIMARY.to_string(), "# Git Style\nUse English Conventional Commits.".to_string()),
        (PEER.to_string(), "# Git Style\nUse English Conventional Commits.\nOld note.".to_string()),
    ]);
    let state = Arc::new(Mutex::new(MockState { files: original.clone(), ..Default::default() }));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let policy = PolicyConfig::default();
    let profile = client.profile_key();

    for missing in [false, true] {
        let (run, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
        let merge = staged.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
        db.update_change_decision_for_profile(&profile, &merge.id, ReviewStatus::Accepted, None).unwrap();
        {
            let mut guard = state.lock().unwrap();
            if missing {
                guard.files.remove(PEER);
                guard.fail_read_uri = Some((PEER, "404 Not Found"));
            } else {
                guard.files.insert(PEER.into(), "Edited during review".into());
            }
        }
        let report = apply_accepted_changes(&client, &db, &policy).await.unwrap();
        assert_eq!(report.conflict_ids, [merge.id.clone()]);
        assert_eq!(db.get_run(&run.id).unwrap().unwrap().status, "conflict");
        let mut guard = state.lock().unwrap();
        assert_eq!(guard.writes, 0);
        guard.files = original.clone();
        guard.fail_read_uri = None;
    }

    let (run, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let merge = staged.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    db.update_change_decision_for_profile(&profile, &merge.id, ReviewStatus::Accepted, None).unwrap();
    state.lock().unwrap().fail_read_uri = Some((PEER, "503 Service Unavailable"));
    let report = apply_accepted_changes(&client, &db, &policy).await.unwrap();
    assert!(report.failed_items[0].1.contains("503"));
    assert!(report.conflict_ids.is_empty());
    assert_eq!(db.list_pending_changes_for_profile(&profile, Some("accepted")).unwrap()[0].id, merge.id);
    {
        let mut guard = state.lock().unwrap();
        assert_eq!(guard.writes, 0);
        guard.fail_read_uri = None;
        guard.fail_move_from = Some(PEER);
    }
    let report = apply_accepted_changes(&client, &db, &policy).await.unwrap();
    assert!(report.applied_ids.is_empty());
    assert!(report.failed_items[0].1.contains("Could not archive merge peer"));
    assert_eq!(state.lock().unwrap().files, original);
    assert_eq!(db.get_run(&run.id).unwrap().unwrap().status, "partial_failure");
}


#[tokio::test]
async fn e2e_auto_apply_low_risk_applies_low_and_keeps_medium_high_pending() {
    const INBOX_NOTE: &str = "viking://~/resources/inbox/scratch.md";
    const PROJECT_README: &str = "viking://resources/projects/demo/README.md";
    const PRIMARY: &str = "viking://~/memories/preferences/git.md";
    const PEER: &str = "viking://~/memories/preferences/git-old.md";

    let state = Arc::new(Mutex::new(MockState {
        files: HashMap::from([
            (INBOX_NOTE.into(), "# Inbox note".into()),
            (PROJECT_README.into(), "# Demo".into()),
            (PRIMARY.into(), "# Git Style\nUse English Conventional Commits.".into()),
            (PEER.into(), "# Git Style\nUse English Conventional Commits.\nOld note.".into()),
        ]),
        merge_score: Some(0.96),
        merge_reflector_response: Some(r#"{"relation":"CONFLICTS","merged_content":"Both positions need review."}"#),
        ..Default::default()
    }));
    let url = start_mock_openviking_server(state.clone()).await;
    let client = test_client(&url, "work");
    let db = DesktopDb::open_in_memory().unwrap();
    let mut policy = PolicyConfig::default();
    policy.lifecycle.dreaming.reflector_mode = ReflectorMode::OpenaiCompatible;
    policy.lifecycle.dreaming.reflector_endpoint = format!("{url}/v1");
    policy.lifecycle.dreaming.reflector_model = "e2e-reflector".into();
    let (_, older) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let older_high = older.iter().find(|c| c.risk_level == RiskLevel::High).unwrap();
    let profile = client.profile_key();
    db.update_change_decision_for_profile(&profile, &older_high.id, ReviewStatus::Accepted, None).unwrap();

    policy.lifecycle.dreaming.reflector_mode = ReflectorMode::Deterministic;
    policy.lifecycle.dreaming.auto_apply_low_risk = true;
    policy.lifecycle.dreaming.auto_snapshot_before_run = false;
    state.lock().unwrap().fail_snapshot = true;
    let (failed_run, failed_staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    assert_eq!(failed_run.status, "ready_for_review");
    assert!(failed_run.snapshot_id.is_none());
    assert!(failed_staged.iter().all(|c| c.status == ReviewStatus::Pending));
    assert!(serde_json::from_str::<Value>(&failed_run.summary_json).unwrap()["auto_apply_error"]
        .as_str().unwrap().contains("Snapshot failed"));
    {
        let mut guard = state.lock().unwrap();
        assert_eq!(guard.writes, 0);
        guard.fail_snapshot = false;
    }

    let (run, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    assert_eq!(run.status, "ready_for_review");
    assert!(run.snapshot_id.is_some(), "Auto-apply must create a rollbackable snapshot");

    let wiki = staged.iter().find(|c| c.action == ChangeAction::WikiCompile).unwrap();
    assert_eq!(wiki.status, ReviewStatus::Applied, "Low-risk WikiCompile should auto-apply");

    let inbox = staged.iter().find(|c| c.action == ChangeAction::MoveFromInbox).unwrap();
    assert_eq!(inbox.status, ReviewStatus::Pending, "Medium-risk MoveFromInbox must stay pending");

    let merge = staged.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    let ev: Value = serde_json::from_str(&merge.evidence_json).unwrap();
    assert_eq!(ev["relation"], "DUPLICATES");
    assert_eq!(merge.risk_level, RiskLevel::Medium);
    assert_eq!(merge.status, ReviewStatus::Pending);
    let accepted = db.list_pending_changes_for_profile(&profile, Some("accepted")).unwrap();
    assert_eq!(accepted.len(), 1);
    assert_eq!(accepted[0].id, older_high.id);

    policy.lifecycle.dreaming.reflector_mode = ReflectorMode::OpenaiCompatible;
    state.lock().unwrap().merge_reflector_response = None;
    let (_, plain) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let merge = plain.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    assert_eq!(merge.risk_level, RiskLevel::Medium);
    assert_eq!(merge.status, ReviewStatus::Pending);

    state.lock().unwrap().merge_reflector_response =
        Some(r##"{"relation":"DUPLICATES","merged_content":"# Classified merged note"}"##);
    state.lock().unwrap().fail_move_from = Some(PEER);
    let (run, failed) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let failed_merge = failed.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    assert_eq!(run.status, "partial_failure");
    assert_eq!(failed_merge.status, ReviewStatus::Conflict);
    assert_eq!(db.list_pending_changes_for_profile(&profile, Some("failed")).unwrap()[0].id, failed_merge.id);
    state.lock().unwrap().fail_move_from = None;

    let (run, classified) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    let merge = classified.iter().find(|c| c.action == ChangeAction::Merge).unwrap();
    assert_eq!(merge.risk_level, RiskLevel::Low);
    assert_eq!(merge.status, ReviewStatus::Applied);
    assert_eq!(merge.snapshot_id, run.snapshot_id);
    let guard = state.lock().unwrap();
    assert!(!parse_frontmatter(&guard.files[PRIMARY]).0.unwrap().verified);
    assert!(!guard.files.contains_key(PEER));
}
