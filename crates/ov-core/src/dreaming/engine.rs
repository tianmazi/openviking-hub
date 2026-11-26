use crate::dreaming::db::{DesktopDb, DreamingRun};
use crate::fs_util::sha256_hex;
use crate::ov_client::{FindRequest, OvClient};
use crate::policy::model::{
    ChangeAction, MemoryCategory, PendingMemoryChange, PolicyConfig, ReviewStatus, RiskLevel,
};
use crate::policy::router::is_protected_rule_path;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReport {
    pub snapshot_id: Option<String>,
    pub applied_ids: Vec<String>,
    pub conflict_ids: Vec<String>,
    pub failed_items: Vec<(String, String)>,
}

/// Execute Stage 1–3 of Dreaming (strictly read-only against remote OpenViking):
/// Harvest -> Analyze (Inbox Sorter, Semantic Dedupe/Conflict Merger, Hot/Cold Promoter & Archiver, Wiki Compile) -> Stage in SQLite.
pub async fn run_dreaming(
    client: &OvClient,
    db: &DesktopDb,
    policy: &PolicyConfig,
    trigger_source: &str,
) -> Result<(DreamingRun, Vec<PendingMemoryChange>), String> {
    let run_id = format!("dream-{}", Uuid::new_v4().simple());
    let mut run = db.create_run(&run_id, trigger_source)?;
    let hit_stats = db.hit_stats_by_uri()?;
    let now = Utc::now().timestamp();
    let stale_cutoff = now - (policy.lifecycle.archival.stale_days as i64 * 86_400);

    let mut staged: Vec<PendingMemoryChange> = Vec::new();
    let mut counts = json!({
        "inbox": 0,
        "merge": 0,
        "promote": 0,
        "archive": 0,
        "wiki_compile": 0
    });

    // Analyzer 1: Inbox Sorter
    if let Ok(inbox_ls) = client.ls(&policy.routing.inbox_uri).await {
        for entry in extract_uri_entries(&inbox_ls) {
            if is_protected_rule_path(policy, &entry) || db.is_pinned(&entry)? {
                continue;
            }
            let content = read_text_safely(client, &entry).await;
            let base_hash = content.as_deref().map(sha256_hex);
            let file_name = entry.rsplit('/').next().unwrap_or("note.md");
            let suggested_dest = format!(
                "{}/projects/general/{}",
                policy.global.default_visibility.resources_prefix(),
                file_name
            );
            let change = PendingMemoryChange {
                id: format!("chg-{}", Uuid::new_v4().simple()),
                run_id: run_id.clone(),
                snapshot_id: None,
                created_at: now,
                action: ChangeAction::MoveFromInbox,
                risk_level: RiskLevel::Medium,
                target_uri: entry.clone(),
                destination_uri: Some(suggested_dest.clone()),
                category: MemoryCategory::from_uri(&entry),
                base_hash,
                before_content: content.clone(),
                after_content: content,
                rationale: format!("Inbox 中发现待分拣条目，建议归类至 {}", suggested_dest),
                evidence_json: json!({ "source": "inbox_sorter", "uri": entry }).to_string(),
                status: ReviewStatus::Pending,
            };
            db.insert_pending_change(&change)?;
            staged.push(change);
            counts["inbox"] = json!(counts["inbox"].as_u64().unwrap_or(0) + 1);
        }
    }

    // Analyzer 2 & 3: Semantic Conflict/Dedupe Merger + Hot Promote + Cold Archival
    let mut seen_merge_pairs: HashSet<(String, String)> = HashSet::new();
    let (mem_root, mem_tree_res) = match client.tree("viking://~/memories/").await {
        Ok(tree) => ("viking://~/memories/", Ok(tree)),
        Err(_) => (
            "viking://user/memories/",
            client.tree("viking://user/memories/").await,
        ),
    };
    if let Ok(mem_tree) = mem_tree_res {
        for uri in extract_uri_entries(&mem_tree) {
            if is_protected_rule_path(policy, &uri) || db.is_pinned(&uri)? {
                continue;
            }
            let category = MemoryCategory::from_uri(&uri);
            let stat = hit_stats.get(&uri);
            let hit_count = stat.map(|s| s.hit_count).unwrap_or(0);
            let last_hit = stat.map(|s| s.last_hit_at).unwrap_or(0);
            let agents = stat.map(|s| s.agents.clone()).unwrap_or_default();

            // 3a. Hot memory promotion: frequently recalled across >=2 distinct agents
            if policy.lifecycle.dreaming.promote_hot_notes && hit_count >= 5 && agents.len() >= 2 {
                let content = read_text_safely(client, &uri).await;
                let base_hash = content.as_deref().map(sha256_hex);
                let change = PendingMemoryChange {
                    id: format!("chg-{}", Uuid::new_v4().simple()),
                    run_id: run_id.clone(),
                    snapshot_id: None,
                    created_at: now,
                    action: ChangeAction::PromoteToIndex,
                    risk_level: RiskLevel::Low,
                    target_uri: uri.clone(),
                    destination_uri: None,
                    category,
                    base_hash,
                    before_content: content.clone(),
                    after_content: content,
                    rationale: format!(
                        "高频跨 Agent 记忆（共命中 {} 次，涉及 {}），建议晋升至顶层 L0/L1 启动索引。",
                        hit_count,
                        agents.join(" / ")
                    ),
                    evidence_json: json!({
                        "hit_count": hit_count,
                        "agents": agents,
                        "last_hit_at": last_hit
                    })
                    .to_string(),
                    status: ReviewStatus::Pending,
                };
                db.insert_pending_change(&change)?;
                staged.push(change);
                counts["promote"] = json!(counts["promote"].as_u64().unwrap_or(0) + 1);
            }

            // 3b. Cold memory archival check
            if policy.lifecycle.archival.enabled
                && hit_count < policy.lifecycle.archival.min_hit_count
                && last_hit < stale_cutoff
            {
                let content = read_text_safely(client, &uri).await;
                let base_hash = content.as_deref().map(sha256_hex);
                let file_name = uri.rsplit('/').next().unwrap_or("memory.md");
                let dest = format!(
                    "{}/{}",
                    policy.lifecycle.archival.archive_uri.trim_end_matches('/'),
                    file_name
                );
                let change = PendingMemoryChange {
                    id: format!("chg-{}", Uuid::new_v4().simple()),
                    run_id: run_id.clone(),
                    snapshot_id: None,
                    created_at: now,
                    action: ChangeAction::Archive,
                    risk_level: RiskLevel::Low,
                    target_uri: uri.clone(),
                    destination_uri: Some(dest),
                    category,
                    base_hash,
                    before_content: content,
                    after_content: None,
                    rationale: format!(
                        "超过 {} 天未达到最低命中次数（当前命中 {} 次），建议归档以减少上下文噪音。",
                        policy.lifecycle.archival.stale_days, hit_count
                    ),
                    evidence_json: json!({
                        "hit_count": hit_count,
                        "last_hit_at": last_hit,
                        "agents": agents
                    })
                    .to_string(),
                    status: ReviewStatus::Pending,
                };
                db.insert_pending_change(&change)?;
                staged.push(change);
                counts["archive"] = json!(counts["archive"].as_u64().unwrap_or(0) + 1);
            }

            // 2. Semantic conflict & dedupe merger (strictly mutable categories only)
            if policy.lifecycle.dreaming.dedupe_and_merge && policy.lifecycle.is_mutable(category) {
                if let Some(content) = read_text_safely(client, &uri).await {
                    let snippet: String = content.chars().take(240).collect();
                    if snippet.trim().len() >= 20 {
                        let scope = format!("{}{}/", mem_root, category.as_str());
                        if let Ok(found) = client
                            .find(&FindRequest {
                                query: snippet,
                                target_uri: Some(scope),
                                limit: Some(4),
                            })
                            .await
                        {
                            for (other_uri, score) in extract_scored_uris(&found) {
                                if other_uri == uri
                                    || score < 0.85
                                    || is_protected_rule_path(policy, &other_uri)
                                {
                                    continue;
                                }
                                let pair = if uri < other_uri {
                                    (uri.clone(), other_uri.clone())
                                } else {
                                    (other_uri.clone(), uri.clone())
                                };
                                if !seen_merge_pairs.insert(pair) {
                                    continue;
                                }
                                let other_content =
                                    read_text_safely(client, &other_uri).await.unwrap_or_default();
                                let merged = build_merged_markdown(&content, &other_content, &other_uri);
                                let change = PendingMemoryChange {
                                    id: format!("chg-{}", Uuid::new_v4().simple()),
                                    run_id: run_id.clone(),
                                    snapshot_id: None,
                                    created_at: now,
                                    action: ChangeAction::Merge,
                                    risk_level: RiskLevel::Medium,
                                    target_uri: uri.clone(),
                                    destination_uri: Some(other_uri.clone()),
                                    category,
                                    base_hash: Some(sha256_hex(&content)),
                                    before_content: Some(content.clone()),
                                    after_content: Some(merged),
                                    rationale: format!(
                                        "检测到与 {} 语义高度重合（相似度 {:.2}），建议合并去重以消除潜在冲突。",
                                        other_uri, score
                                    ),
                                    evidence_json: json!({
                                        "source": "semantic_dedupe",
                                        "peer_uri": other_uri,
                                        "similarity": score,
                                        "category": category.as_str()
                                    })
                                    .to_string(),
                                    status: ReviewStatus::Pending,
                                };
                                db.insert_pending_change(&change)?;
                                staged.push(change);
                                counts["merge"] = json!(counts["merge"].as_u64().unwrap_or(0) + 1);
                            }
                        }
                    }
                }
            }
        }
    }

    // Analyzer 4: Wiki Compile Proposal
    if policy.lifecycle.dreaming.compile_wiki {
        if let Ok(proj_ls) = client.ls("viking://resources/projects/").await {
            let projects = extract_uri_entries(&proj_ls);
            if !projects.is_empty() {
                let change = PendingMemoryChange {
                    id: format!("chg-{}", Uuid::new_v4().simple()),
                    run_id: run_id.clone(),
                    snapshot_id: None,
                    created_at: now,
                    action: ChangeAction::WikiCompile,
                    risk_level: RiskLevel::Low,
                    target_uri: "viking://resources/projects/".to_string(),
                    destination_uri: None,
                    category: MemoryCategory::Patterns,
                    base_hash: None,
                    before_content: None,
                    after_content: Some(format!(
                        "触发 OpenViking Compile 任务，编译 {} 个项目目录为互联 Wiki。",
                        projects.len()
                    )),
                    rationale: "将近期积累的项目文档与经验编译为结构化 Wiki，降低检索 Token 消耗。"
                        .to_string(),
                    evidence_json: json!({ "project_count": projects.len(), "projects": projects })
                        .to_string(),
                    status: ReviewStatus::Pending,
                };
                db.insert_pending_change(&change)?;
                staged.push(change);
                counts["wiki_compile"] = json!(1);
            }
        }
    }

    let summary_str = counts.to_string();
    let final_status = if policy.lifecycle.dreaming.require_human_review {
        "ready_for_review"
    } else {
        "applied"
    };
    db.update_run_status(&run_id, final_status, None, Some(&summary_str))?;
    run.status = final_status.to_string();
    run.summary_json = summary_str;
    run.finished_at = Some(Utc::now().timestamp());

    Ok((run, staged))
}

/// Stage 4: Atomically commit all `accepted` changes:
/// 1. Create remote snapshot (`POST /api/v1/snapshot/commit`) if enabled.
/// 2. Verify SHA-256 `base_hash` against current remote content (optimistic concurrency).
/// 3. Apply content writes, moves, or wiki compilation.
pub async fn apply_accepted_changes(
    client: &OvClient,
    db: &DesktopDb,
    policy: &PolicyConfig,
) -> Result<ApplyReport, String> {
    let accepted = db.list_pending_changes(Some("accepted"))?;
    if accepted.is_empty() {
        return Ok(ApplyReport {
            snapshot_id: None,
            applied_ids: Vec::new(),
            conflict_ids: Vec::new(),
            failed_items: Vec::new(),
        });
    }

    let mut snapshot_id: Option<String> = None;
    if policy.lifecycle.dreaming.auto_snapshot_before_run {
        let label = format!("dreaming-commit-{}", Utc::now().format("%Y%m%dT%H%M%SZ"));
        if let Ok(snap) = client.snapshot_commit(&label, None).await {
            snapshot_id = snap
                .get("snapshot_id")
                .or_else(|| snap.get("id"))
                .or_else(|| snap.get("commit_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }

    let mut applied_ids = Vec::new();
    let mut conflict_ids = Vec::new();
    let mut failed_items = Vec::new();

    for chg in accepted {
        if is_protected_rule_path(policy, &chg.target_uri) {
            failed_items.push((
                chg.id.clone(),
                format!("Blocked: {} matches protected_rule_globs", chg.target_uri),
            ));
            continue;
        }

        // Optimistic concurrency check
        if let Some(expected_hash) = &chg.base_hash {
            if let Some(live_content) = read_text_safely(client, &chg.target_uri).await {
                if sha256_hex(&live_content) != *expected_hash {
                    db.mark_change_applied(
                        &chg.id,
                        snapshot_id.as_deref(),
                        ReviewStatus::Conflict,
                    )?;
                    conflict_ids.push(chg.id.clone());
                    continue;
                }
            }
        }

        let exec_res = match chg.action {
            ChangeAction::Merge | ChangeAction::PromoteToIndex | ChangeAction::DemoteFromIndex => {
                let body = chg.after_content.as_deref().unwrap_or("");
                client.write_content(&chg.target_uri, body).await
            }
            ChangeAction::MoveFromInbox | ChangeAction::Archive => {
                if let Some(dest) = &chg.destination_uri {
                    client.mv(&chg.target_uri, dest).await
                } else {
                    Err(crate::ov_client::OvClientError::Config(
                        "missing destination_uri".to_string(),
                    ))
                }
            }
            ChangeAction::WikiCompile => {
                client
                    .trigger_compile("desktop-dreaming-wiki", vec![chg.target_uri.clone()])
                    .await
            }
        };

        match exec_res {
            Ok(_) => {
                db.mark_change_applied(&chg.id, snapshot_id.as_deref(), ReviewStatus::Applied)?;
                db.update_run_status(&chg.run_id, "applied", snapshot_id.as_deref(), None)?;
                applied_ids.push(chg.id.clone());
            }
            Err(e) => {
                failed_items.push((chg.id.clone(), e.to_string()));
            }
        }
    }

    Ok(ApplyReport {
        snapshot_id,
        applied_ids,
        conflict_ids,
        failed_items,
    })
}

/// Roll back a previously applied Dreaming run via `POST /api/v1/snapshot/restore`.
pub async fn rollback_dreaming_run(
    client: &OvClient,
    db: &DesktopDb,
    run_id: &str,
) -> Result<Value, String> {
    let run = db
        .get_run(run_id)?
        .ok_or_else(|| format!("Dreaming run not found: {}", run_id))?;
    let snap_id = run
        .snapshot_id
        .ok_or_else(|| format!("Run {} has no associated snapshot_id", run_id))?;
    let res = client
        .snapshot_restore(&snap_id)
        .await
        .map_err(|e| e.to_string())?;
    db.update_run_status(run_id, "rolled_back", Some(&snap_id), None)?;
    Ok(res)
}

async fn read_text_safely(client: &OvClient, uri: &str) -> Option<String> {
    let val = client.read_content(uri, "l2").await.ok()?;
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    val.get("content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

fn extract_uri_entries(val: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(uri) = item.get("uri").and_then(|u| u.as_str()) {
                let is_dir = item.get("isDir").and_then(|b| b.as_bool()).unwrap_or(false);
                if !is_dir {
                    out.push(uri.to_string());
                }
            } else if let Some(s) = item.as_str() {
                out.push(s.to_string());
            }
        }
    } else if let Some(items) = val.get("items").and_then(|i| i.as_array()) {
        for item in items {
            if let Some(uri) = item.get("uri").and_then(|u| u.as_str()) {
                out.push(uri.to_string());
            }
        }
    }
    out
}

fn extract_scored_uris(val: &Value) -> Vec<(String, f64)> {
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
    let mut out = Vec::new();
    for items in slices {
        for item in items {
            let uri = item.get("uri").and_then(|u| u.as_str());
            let score = item
                .get("score")
                .or_else(|| item.get("similarity"))
                .and_then(|s| s.as_f64())
                .unwrap_or(0.0);
            if let Some(u) = uri {
                out.push((u.to_string(), score));
            }
        }
    }
    out
}

fn build_merged_markdown(primary: &str, peer: &str, peer_uri: &str) -> String {
    let mut seen: HashSet<String> = HashSet::new();
    let mut lines: Vec<String> = Vec::new();
    for line in primary.lines() {
        seen.insert(line.trim().to_string());
        lines.push(line.to_string());
    }
    let mut extra: Vec<String> = Vec::new();
    for line in peer.lines() {
        let key = line.trim().to_string();
        if !key.is_empty() && !seen.contains(&key) {
            seen.insert(key);
            extra.push(line.to_string());
        }
    }
    if !extra.is_empty() {
        lines.push(String::new());
        lines.push(format!("<!-- Merged from {} -->", peer_uri));
        lines.extend(extra);
    }
    lines.join("\n")
}
