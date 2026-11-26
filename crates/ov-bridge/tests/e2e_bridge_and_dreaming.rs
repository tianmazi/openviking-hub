use ov_core::dreaming::db::DesktopDb;
use ov_core::dreaming::engine::{apply_accepted_changes, rollback_dreaming_run, run_dreaming};
use ov_core::ov_client::{OvCliConfig, OvClient};
use ov_core::policy::model::{ChangeAction, PolicyConfig, ReviewStatus};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockState {
    files: HashMap<String, String>,
    snapshots: Vec<String>,
    restored: Vec<String>,
    compiled: Vec<String>,
}

async fn start_mock_openviking_server(state: Arc<Mutex<MockState>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let st = state.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 32768];
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                let req_text = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_text.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let full_path = parts.next().unwrap_or("/");
                let (path, query) = full_path.split_once('?').unwrap_or((full_path, ""));
                let body_str = req_text.split("\r\n\r\n").nth(1).unwrap_or("");
                let body_json: Value = serde_json::from_str(body_str).unwrap_or(json!({}));

                let uri_param = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("uri="))
                    .map(|v| {
                        v.replace("%3A", ":")
                            .replace("%2F", "/")
                            .replace("%7E", "~")
                    })
                    .unwrap_or_default();

                let resp_json: Value = match (method, path) {
                    ("GET", "/health") => json!({ "status": "ok", "result": { "healthy": true } }),
                    ("GET", "/api/v1/system/status") => {
                        json!({ "status": "ok", "result": { "version": "1.0.0-e2e" } })
                    }
                    ("GET", "/api/v1/observer/system") => {
                        json!({ "status": "ok", "result": { "queue": "idle", "vikingdb": "ready" } })
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
                        let items: Vec<Value> = guard
                            .files
                            .keys()
                            .filter(|k| k.starts_with(&uri_param))
                            .map(|k| json!({ "uri": k, "isDir": false }))
                            .collect();
                        json!({ "status": "ok", "result": items })
                    }
                    ("GET", "/api/v1/content/read") => {
                        let guard = st.lock().unwrap();
                        let content = guard.files.get(&uri_param).cloned().unwrap_or_default();
                        json!({ "status": "ok", "result": { "content": content } })
                    }
                    ("GET", "/api/v1/content/overview") => {
                        json!({ "status": "ok", "result": { "content": format!("L1_OVERVIEW_FOR:{}", uri_param) } })
                    }
                    ("GET", "/api/v1/content/abstract") => {
                        json!({ "status": "ok", "result": { "content": format!("L0_ABSTRACT_FOR:{}", uri_param) } })
                    }
                    ("POST", "/api/v1/content/write") => {
                        let uri = body_json["uri"].as_str().unwrap_or("").to_string();
                        let content = body_json["content"].as_str().unwrap_or("").to_string();
                        st.lock().unwrap().files.insert(uri.clone(), content);
                        json!({ "status": "ok", "result": { "uri": uri, "written": true } })
                    }
                    ("POST", "/api/v1/fs/mv") => {
                        let from = body_json["from_uri"].as_str().unwrap_or("");
                        let to = body_json["to_uri"].as_str().unwrap_or("").to_string();
                        let mut guard = st.lock().unwrap();
                        if let Some(v) = guard.files.remove(from) {
                            guard.files.insert(to.clone(), v);
                        }
                        json!({ "status": "ok", "result": { "moved_to": to } })
                    }
                    ("POST", "/api/v1/search/search") => {
                        json!({
                            "status": "ok",
                            "result": {
                                "items": [
                                    { "uri": "viking://user/memories/preferences/git.md", "score": 0.92, "snippet": "Use Conventional Commits" }
                                ]
                            }
                        })
                    }
                    ("POST", "/api/v1/search/find") => {
                        json!({
                            "status": "ok",
                            "result": {
                                "items": [
                                    { "uri": "viking://user/memories/preferences/git-old.md", "score": 0.89 }
                                ]
                            }
                        })
                    }
                    ("POST", "/api/v1/snapshot/commit") => {
                        let snap = format!("snap-e2e-{}", st.lock().unwrap().snapshots.len() + 1);
                        st.lock().unwrap().snapshots.push(snap.clone());
                        json!({ "status": "ok", "result": { "snapshot_id": snap } })
                    }
                    ("POST", "/api/v1/snapshot/restore") => {
                        let snap = body_json["snapshot_id"].as_str().unwrap_or("").to_string();
                        st.lock().unwrap().restored.push(snap.clone());
                        json!({ "status": "ok", "result": { "restored": snap } })
                    }
                    ("POST", "/api/v1/compile") => {
                        let name = body_json["compile_name"].as_str().unwrap_or("").to_string();
                        st.lock().unwrap().compiled.push(name.clone());
                        json!({ "status": "ok", "result": { "task_id": "task-e2e-1" } })
                    }
                    _ => json!({ "status": "ok", "result": {} }),
                };

                let body_bytes = serde_json::to_vec(&resp_json).unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body_bytes.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body_bytes).await;
            });
        }
    });

    format!("http://{}", addr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_ov_bridge_mcp_and_dreaming_pipeline() {
    let state = Arc::new(Mutex::new(MockState::default()));
    {
        let mut guard = state.lock().unwrap();
        guard.files.insert(
            "viking://~/resources/inbox/scratch.md".to_string(),
            "# Scratch note\nAuth token header notes".to_string(),
        );
        guard.files.insert(
            "viking://user/memories/preferences/git.md".to_string(),
            "# Git Commit Style\nAlways use English Conventional Commits for clarity.".to_string(),
        );
        guard.files.insert(
            "viking://user/memories/preferences/git-old.md".to_string(),
            "# Git Commit Style\nAlways use English Conventional Commits for clarity.\nExtra legacy line.".to_string(),
        );
        guard.files.insert(
            "viking://resources/projects/demo/README.md".to_string(),
            "# Demo Project".to_string(),
        );
    }

    let base_url = start_mock_openviking_server(state.clone()).await;

    // 1. E2E test the compiled `ov-bridge` binary over stdio JSON-RPC 2.0
    let tmp = tempdir().unwrap();
    let conf_path = tmp.path().join("ovcli.conf");
    std::fs::write(
        &conf_path,
        serde_json::to_string(&json!({ "url": base_url, "api_key": "user-test-key" })).unwrap(),
    )
    .unwrap();

    let bridge_bin = env!("CARGO_BIN_EXE_ov-bridge");
    let mut child = Command::new(bridge_bin)
        .args(["mcp", "--agent", "claude-code"])
        .env("OPENVIKING_CLI_CONFIG_FILE", &conf_path)
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn ov-bridge");

    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })
        )
        .unwrap();
        // Recall should auto-expand L0 -> L1 because mock score is 0.92 >= 0.80
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "recall", "arguments": { "query": "git commit style" } }
            })
        )
        .unwrap();
        // Protected rule glob write should be denied
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "remember",
                    "arguments": { "uri": "viking://resources/projects/demo/CLAUDE.md", "content": "hack" }
                }
            })
        )
        .unwrap();
        // Normal remember write should succeed
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "remember",
                    "arguments": { "uri": "viking://user/memories/preferences/editor.md", "content": "Prefer Rust + Tauri" }
                }
            })
        )
        .unwrap();
    }

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 4);

    // Verify L0 -> L1 auto-expansion fired in recall
    let recall_text = responses[1]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        recall_text.contains("Auto-Expanded L1 Overview"),
        "Expected L0->L1 auto-expansion in recall output, got: {}",
        recall_text
    );

    // Verify protected CLAUDE.md write was rejected
    assert_eq!(responses[2]["result"]["isError"], json!(true));

    // Verify normal remember persisted to mock OpenViking
    assert!(state
        .lock()
        .unwrap()
        .files
        .contains_key("viking://user/memories/preferences/editor.md"));

    // 2. Full Dreaming Stage-then-Commit + Optimistic Lock + Snapshot Rollback E2E
    let client = OvClient::new(&OvCliConfig {
        url: base_url,
        api_key: "user-test-key".to_string(),
        ..Default::default()
    })
    .unwrap();
    let db = DesktopDb::open_in_memory().unwrap();
    let policy = PolicyConfig::default();

    let (run, staged) = run_dreaming(&client, &db, &policy, "e2e").await.unwrap();
    assert_eq!(run.status, "ready_for_review");
    assert!(staged.iter().any(|c| c.action == ChangeAction::MoveFromInbox));
    assert!(staged.iter().any(|c| c.action == ChangeAction::Merge));
    assert!(staged.iter().any(|c| c.action == ChangeAction::WikiCompile));

    // Simulate concurrent remote drift on git.md before apply
    state.lock().unwrap().files.insert(
        "viking://user/memories/preferences/git.md".to_string(),
        "# Modified concurrently by another agent!".to_string(),
    );

    // Approve all staged changes
    for c in &staged {
        db.update_change_decision(&c.id, ReviewStatus::Accepted, None)
            .unwrap();
    }

    let report = apply_accepted_changes(&client, &db, &policy).await.unwrap();
    assert!(report.snapshot_id.is_some(), "Expected snapshot before commit");
    assert!(
        !report.conflict_ids.is_empty(),
        "Expected optimistic concurrency conflict on drifted git.md"
    );
    assert!(
        !report.applied_ids.is_empty(),
        "Expected non-drifted changes to apply cleanly"
    );

    // Roll back the Dreaming run via snapshot_restore
    rollback_dreaming_run(&client, &db, &run.id).await.unwrap();
    assert_eq!(state.lock().unwrap().restored.len(), 1);
}
