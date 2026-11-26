use crate::dreaming::db::{DesktopDb, DreamingRun};
use crate::dreaming::reflector::{reflect_inbox_route, reflect_merge};
use crate::fs_util::sha256_hex;
use crate::ov_client::{snapshot_commit_id, FindRequest, OvClient, OvClientError};
use crate::policy::model::{
    ChangeAction, MemoryCategory, PendingMemoryChange, PolicyConfig, ReviewStatus, RiskLevel,
};
use crate::policy::router::{is_protected_rule_path, USER_MEMORIES_URI};
use crate::provenance::{
    is_write_gate_verified, render_frontmatter, split_frontmatter, MemoryProvenance,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};
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
    let profile_key = client.profile_key();
    let mem_tree = match client.tree(USER_MEMORIES_URI).await {
        Ok(tree) => Some(tree),
        Err(e) if is_not_found(&e) => None,
        Err(e) => return Err(e.to_string()),
    };
    let hit_stats = db.hit_stats_by_uri_for_profile(&profile_key)?;
    let run_id = format!("dream-{}", Uuid::new_v4().simple());
    let mut run = db.create_run_for_profile(&profile_key, &run_id, trigger_source)?;
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

    let project_root = format!(
        "{}/projects/",
        policy.global.default_visibility.resources_prefix()
    );
    let available_projects = list_project_dirs(client, &project_root).await;

    // Analyzer 1: Inbox Sorter (uses optional LLM/CLI reflector when enabled)
    if let Ok(inbox_ls) = client.ls(&policy.routing.inbox_uri).await {
        for entry in listing_entries(&inbox_ls).into_iter().filter_map(file_uri) {
            if is_protected_rule_path(policy, entry)
                || db.is_pinned_for_profile(&profile_key, entry)?
            {
                continue;
            }
            let content = read_text(client, entry).await.ok();
            let base_hash = content.as_deref().map(sha256_hex);
            let file_name = entry.rsplit('/').next().unwrap_or("note.md");
            let default_dest = format!("{project_root}general/{file_name}");

            let (suggested_dest, rationale, reflector_label) = if let Some(ref raw) = content {
                if let Some(res) = reflect_inbox_route(
                    &policy.lifecycle.dreaming,
                    entry,
                    raw,
                    &available_projects,
                    &default_dest,
                )
                .await
                {
                    (
                        res.destination_uri,
                        format!("{}（{}）", res.rationale, res.engine_label),
                        res.engine_label,
                    )
                } else {
                    (
                        default_dest.clone(),
                        format!("Inbox 中发现待分拣条目，建议归类至 {}", default_dest),
                        "deterministic".to_string(),
                    )
                }
            } else {
                (
                    default_dest.clone(),
                    format!("Inbox 中发现待分拣条目，建议归类至 {}", default_dest),
                    "deterministic".to_string(),
                )
            };

            let change = PendingMemoryChange {
                id: format!("chg-{}", Uuid::new_v4().simple()),
                run_id: run_id.clone(),
                snapshot_id: None,
                created_at: now,
                action: ChangeAction::MoveFromInbox,
                risk_level: RiskLevel::Medium,
                target_uri: entry.to_string(),
                destination_uri: Some(suggested_dest),
                category: MemoryCategory::from_uri(entry),
                base_hash,
                before_content: content.clone(),
                after_content: content,
                rationale,
                evidence_json: json!({
                    "source": "inbox_sorter",
                    "uri": entry,
                    "reflector": reflector_label
                })
                .to_string(),
                status: ReviewStatus::Pending,
            };
            db.insert_pending_change_for_profile(&profile_key, &change)?;
            staged.push(change);
            counts["inbox"] = json!(counts["inbox"].as_u64().unwrap_or(0) + 1);
        }
    }

    // Analyzer 2 & 3: Semantic Conflict/Dedupe Merger + Hot Promote (with Write-Gate) + Cold Archival
    let mut seen_merge_pairs: HashSet<(String, String)> = HashSet::new();
    if let Some(mem_tree) = mem_tree {
        for item in listing_entries(&mem_tree) {
            let Some(uri) = file_uri(item) else {
                continue;
            };
            if is_protected_rule_path(policy, uri) || db.is_pinned_for_profile(&profile_key, uri)? {
                continue;
            }
            let category = MemoryCategory::from_uri(uri);
            let stat = hit_stats.get(uri);
            let hit_count = stat.map(|s| s.hit_count).unwrap_or(0);
            let last_hit = stat.map(|s| s.last_hit_at).unwrap_or(0);
            let agents = stat.map(|s| s.agents.clone()).unwrap_or_default();

            // 3a. Hot memory promotion with Anti-Contagion Write-Gate
            if policy.lifecycle.dreaming.promote_hot_notes && hit_count >= 5 && agents.len() >= 2 {
                let content = read_text(client, uri).await.ok();
                let gate_ok = content
                    .as_deref()
                    .map(|raw| is_write_gate_verified(raw, i64::from(hit_count), agents.len()))
                    .unwrap_or(false);
                if gate_ok {
                    let base_hash = content.as_deref().map(sha256_hex);
                    let change = PendingMemoryChange {
                        id: format!("chg-{}", Uuid::new_v4().simple()),
                        run_id: run_id.clone(),
                        snapshot_id: None,
                        created_at: now,
                        action: ChangeAction::PromoteToIndex,
                        risk_level: RiskLevel::Low,
                        target_uri: uri.to_string(),
                        destination_uri: None,
                        category,
                        base_hash,
                        before_content: content.clone(),
                        after_content: content,
                        rationale: format!(
                            "高频跨 Agent 记忆（共命中 {} 次，涉及 {}，已通过 Write-Gate 校验），建议晋升至顶层 L0/L1 启动索引。",
                            hit_count,
                            agents.join(" / ")
                        ),
                        evidence_json: json!({
                            "hit_count": hit_count,
                            "agents": agents,
                            "last_hit_at": last_hit,
                            "write_gate_passed": true
                        })
                        .to_string(),
                        status: ReviewStatus::Pending,
                    };
                    db.insert_pending_change_for_profile(&profile_key, &change)?;
                    staged.push(change);
                    counts["promote"] = json!(counts["promote"].as_u64().unwrap_or(0) + 1);
                }
            }

            // 3b. Cold memory archival check
            let mod_time_ts = item
                .get("modTime")
                .and_then(Value::as_str)
                .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
                .map(|time| time.timestamp());
            if policy.lifecycle.archival.enabled
                && hit_count < policy.lifecycle.archival.min_hit_count
                && ((hit_count > 0 && last_hit < stale_cutoff)
                    || (hit_count == 0 && mod_time_ts.is_some_and(|ts| ts < stale_cutoff)))
            {
                let content = read_text(client, uri).await.ok();
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
                    target_uri: uri.to_string(),
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
                        "mod_time": mod_time_ts,
                        "agents": agents
                    })
                    .to_string(),
                    status: ReviewStatus::Pending,
                };
                db.insert_pending_change_for_profile(&profile_key, &change)?;
                staged.push(change);
                counts["archive"] = json!(counts["archive"].as_u64().unwrap_or(0) + 1);
            }

            // 2. Semantic conflict & dedupe merger (strictly mutable categories only)
            if policy.lifecycle.dreaming.dedupe_and_merge && policy.lifecycle.is_mutable(category) {
                if let Ok(content) = read_text(client, uri).await {
                    let snippet: String = content.chars().take(240).collect();
                    if snippet.trim().len() >= 20 {
                        let scope = format!("{USER_MEMORIES_URI}{}/", category.as_str());
                        if let Ok(found) = client
                            .find(&FindRequest {
                                query: snippet,
                                target_uri: Some(scope),
                                limit: Some(4),
                            })
                            .await
                        {
                            for (other_uri, score) in extract_scored_uris(&found) {
                                if other_uri.as_str() == uri
                                    || score < 0.85
                                    || is_protected_rule_path(policy, &other_uri)
                                {
                                    continue;
                                }
                                let pair = if uri < other_uri.as_str() {
                                    (uri.to_string(), other_uri.clone())
                                } else {
                                    (other_uri.clone(), uri.to_string())
                                };
                                if !seen_merge_pairs.insert(pair) {
                                    continue;
                                }
                                let Ok(other_content) = read_text(client, &other_uri).await
                                else {
                                    continue;
                                };

                                let (frontmatter, primary_body) = split_frontmatter(&content);
                                let (_, peer_body) = split_frontmatter(&other_content);
                                let (merged_body, reflector_label) = match reflect_merge(
                                    &policy.lifecycle.dreaming,
                                    uri,
                                    primary_body,
                                    &other_uri,
                                    peer_body,
                                )
                                .await
                                {
                                    Some(res) => (res.content, res.engine_label),
                                    None => (
                                        build_merged_markdown(primary_body, peer_body, &other_uri),
                                        "deterministic".to_string(),
                                    ),
                                };

                                let mut prov = MemoryProvenance::new(
                                    "dreaming",
                                    "shared",
                                    true,
                                    score as f32,
                                );
                                prov.updated_by = format!("dreaming:{}", reflector_label);
                                prov.evidence_uris = vec![uri.to_string(), other_uri.clone()];
                                let merged_with_prov =
                                    render_frontmatter(frontmatter, &merged_body, &prov);

                                let change = PendingMemoryChange {
                                    id: format!("chg-{}", Uuid::new_v4().simple()),
                                    run_id: run_id.clone(),
                                    snapshot_id: None,
                                    created_at: now,
                                    action: ChangeAction::Merge,
                                    risk_level: RiskLevel::Medium,
                                    target_uri: uri.to_string(),
                                    destination_uri: Some(other_uri.clone()),
                                    category,
                                    base_hash: Some(sha256_hex(&content)),
                                    before_content: Some(content.clone()),
                                    after_content: Some(merged_with_prov),
                                    rationale: format!(
                                        "检测到与 {} 语义高度重合（相似度 {:.2}），已通过 {} 生成去重合并提案。",
                                        other_uri, score, reflector_label
                                    ),
                                    evidence_json: json!({
                                        "source": "semantic_dedupe",
                                        "peer_uri": other_uri,
                                        "similarity": score,
                                        "category": category.as_str(),
                                        "reflector": reflector_label
                                    })
                                    .to_string(),
                                    status: ReviewStatus::Pending,
                                };
                                db.insert_pending_change_for_profile(&profile_key, &change)?;
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
        let wiki_project_root = "viking://resources/projects/";
        let wiki_projects = if project_root == wiki_project_root {
            available_projects
        } else {
            list_project_dirs(client, wiki_project_root).await
        };
        if !wiki_projects.is_empty() {
            let change = PendingMemoryChange {
                id: format!("chg-{}", Uuid::new_v4().simple()),
                run_id: run_id.clone(),
                snapshot_id: None,
                created_at: now,
                action: ChangeAction::WikiCompile,
                risk_level: RiskLevel::Low,
                target_uri: wiki_project_root.to_string(),
                destination_uri: Some("viking://resources/desktop-dreaming-wiki/".to_string()),
                category: MemoryCategory::Patterns,
                base_hash: None,
                before_content: None,
                after_content: Some(format!(
                    "触发 OpenViking Compile 任务，编译 {} 个项目目录为互联 Wiki。",
                    wiki_projects.len()
                )),
                rationale: "将近期积累的项目文档与经验编译为结构化 Wiki，降低检索 Token 消耗。"
                    .to_string(),
                evidence_json: json!({
                    "project_count": wiki_projects.len(),
                    "projects": wiki_projects
                })
                .to_string(),
                status: ReviewStatus::Pending,
            };
            db.insert_pending_change_for_profile(&profile_key, &change)?;
            staged.push(change);
            counts["wiki_compile"] = json!(1);
        }
    }

    let summary_str = counts.to_string();
    let final_status = "ready_for_review";
    db.update_run_status_for_profile(&profile_key, &run_id, final_status, None, Some(&summary_str))?;
    run.status = final_status.to_string();
    run.summary_json = summary_str;
    run.finished_at = Some(Utc::now().timestamp());

    Ok((run, staged))
}

/// Stage 4: Claim and apply this profile's accepted changes:
/// 1. Create remote snapshot (`POST /api/v1/snapshot/commit`) if enabled.
/// 2. Verify SHA-256 `base_hash` against current remote content (optimistic concurrency).
/// 3. Apply content writes, moves, or wiki compilation.
pub async fn apply_accepted_changes(
    client: &OvClient,
    db: &DesktopDb,
    policy: &PolicyConfig,
) -> Result<ApplyReport, String> {
    let profile_key = client.profile_key();
    let claimed = db.claim_accepted_changes_for_profile(&profile_key)?;
    let snapshot_paths: Vec<String> = claimed
        .iter()
        .flat_map(|change| match change.action {
            ChangeAction::WikiCompile => change.destination_uri.iter().map(String::as_str).collect::<Vec<_>>(),
            ChangeAction::MoveFromInbox | ChangeAction::Archive => {
                std::iter::once(change.target_uri.as_str())
                    .chain(change.destination_uri.as_deref())
                    .collect()
            }
            _ => vec![change.target_uri.as_str()],
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let snapshot_id = if policy.lifecycle.dreaming.auto_snapshot_before_run && !claimed.is_empty() {
        let label = format!("dreaming-commit-{}", Utc::now().format("%Y%m%dT%H%M%SZ"));
        let snapshot = client
            .snapshot_commit(&label, Some(snapshot_paths.clone()))
            .await
            .map_err(|e| e.to_string())
            .and_then(|value| {
                snapshot_commit_id(&value)
                    .map(str::to_owned)
                    .ok_or_else(|| "response contained no snapshot ID".to_string())
            });
        match snapshot {
            Ok(id) => Some(id),
            Err(e) => {
                if let Err(release_err) = db.release_claim_for_profile(&profile_key, &claimed) {
                    return Err(format!(
                        "Snapshot failed: {e}; could not release claims: {release_err}"
                    ));
                }
                return Err(format!("Snapshot failed: {e}"));
            }
        }
    } else {
        None
    };

    let mut applied_ids = Vec::new();
    let mut conflict_ids = Vec::new();
    let mut failed_items = Vec::new();

    for (index, chg) in claimed.iter().enumerate() {
        let matches_base = match (chg.action, chg.base_hash.as_deref()) {
            (ChangeAction::WikiCompile, _) => true,
            (_, Some(expected)) => match read_text(client, &chg.target_uri).await {
                Ok(live) => sha256_hex(&live) == expected,
                Err(error) => {
                    db.release_claim_for_profile(&profile_key, std::slice::from_ref(chg))?;
                    failed_items.push((chg.id.clone(), error));
                    continue;
                }
            },
            (_, None) => false,
        };
        let protection = check_write_protection(db, policy, &profile_key, chg);
        let blocked = match protection {
            Ok(blocked) => blocked,
            Err(e) => {
                db.release_claim_for_profile(&profile_key, &claimed[index..])?;
                return Err(e);
            }
        };
        if let Some(uri) = blocked {
            db.finish_claimed_change_for_profile(
                &profile_key,
                chg,
                None,
                ReviewStatus::Conflict.as_str(),
            )?;
            failed_items.push((
                chg.id.clone(),
                format!("Blocked: {uri} is protected or pinned"),
            ));
            continue;
        }

        if !matches_base {
            db.finish_claimed_change_for_profile(
                &profile_key,
                chg,
                None,
                ReviewStatus::Conflict.as_str(),
            )?;
            conflict_ids.push(chg.id.clone());
            continue;
        }

        match execute_change(client, chg).await {
            Ok(()) => {
                db.finish_claimed_change_for_profile(
                    &profile_key,
                    chg,
                    snapshot_id.as_deref(),
                    ReviewStatus::Applied.as_str(),
                )?;
                applied_ids.push(chg.id.clone());
            }
            Err(e) => {
                db.finish_claimed_change_for_profile(
                    &profile_key,
                    chg,
                    snapshot_id.as_deref(),
                    "failed",
                )?;
                failed_items.push((chg.id.clone(), e));
            }
        }
    }

    if snapshot_id.is_some() && !applied_ids.is_empty() && !snapshot_paths.is_empty() {
        let post_label = format!("dreaming-applied-{}", Utc::now().format("%Y%m%dT%H%M%SZ"));
        if let Err(error) = client.snapshot_commit(&post_label, Some(snapshot_paths)).await {
            failed_items.push(("post-apply-snapshot".to_string(), error.to_string()));
        }
    }

    Ok(ApplyReport {
        snapshot_id,
        applied_ids,
        conflict_ids,
        failed_items,
    })
}

fn check_write_protection<'a>(
    db: &DesktopDb,
    policy: &PolicyConfig,
    profile_key: &str,
    change: &'a PendingMemoryChange,
) -> Result<Option<&'a str>, String> {
    for uri in std::iter::once(change.target_uri.as_str()).chain(change.destination_uri.as_deref())
    {
        if is_protected_rule_path(policy, uri) || db.is_pinned_for_profile(profile_key, uri)? {
            return Ok(Some(uri));
        }
    }
    Ok(None)
}

async fn execute_change(client: &OvClient, change: &PendingMemoryChange) -> Result<(), String> {
    let result = match change.action {
        ChangeAction::Merge | ChangeAction::PromoteToIndex | ChangeAction::DemoteFromIndex => {
            let body = change
                .after_content
                .as_deref()
                .ok_or("missing after_content")?;
            client.write_content(&change.target_uri, body).await
        }
        ChangeAction::MoveFromInbox | ChangeAction::Archive => {
            let dest = change
                .destination_uri
                .as_deref()
                .ok_or("missing destination_uri")?;
            client.mv(&change.target_uri, dest).await
        }
        ChangeAction::WikiCompile => {
            let dest = change
                .destination_uri
                .as_deref()
                .ok_or("missing Wiki destination URI")?;
            let evidence: Value =
                serde_json::from_str(&change.evidence_json).map_err(|e| e.to_string())?;
            let projects = evidence
                .get("projects")
                .and_then(Value::as_array)
                .ok_or("missing Wiki source directories")?;
            let sources = projects.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            client.trigger_compile(dest, sources).await
        }
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

/// Reverse the files and Wiki output changed by a previously applied Dreaming run.
pub async fn rollback_dreaming_run(
    client: &OvClient,
    db: &DesktopDb,
    run_id: &str,
) -> Result<Value, String> {
    let profile_key = client.profile_key();
    let run = db
        .get_run(run_id)?
        .ok_or_else(|| format!("Dreaming run not found: {}", run_id))?;
    if run.profile_key.is_empty() || run.profile_key != profile_key {
        return Err(format!("Run {run_id} does not belong to the active profile"));
    }
    if run.status == "applying" {
        return Err(format!("Run {run_id} is applying changes"));
    }
    let snap_id = run
        .snapshot_id
        .ok_or_else(|| format!("Run {} has no associated snapshot_id", run_id))?;
    let mut changes = db.list_applied_changes_for_run(&profile_key, run_id)?;
    if changes.is_empty() {
        return Err(format!("Run {run_id} has no applied changes"));
    }
    // Reverse the order used by claim_accepted_changes_for_profile.
    changes.sort_unstable_by(|a, b| a.created_at.cmp(&b.created_at).then(b.id.cmp(&a.id)));
    let mut restored = Vec::with_capacity(changes.len());
    for change in &changes {
        let change_snapshot = change.snapshot_id.as_deref().unwrap_or(&snap_id);
        let response = rollback_change(client, change_snapshot, change)
            .await
            .map_err(|e| format!("Restore failed for {}: {e}", change.target_uri))?;
        restored.push(response);
    }
    db.update_run_status_for_profile(&profile_key, run_id, "rolled_back", Some(&snap_id), None)?;
    match restored.len() {
        1 => Ok(restored.remove(0)),
        _ => Ok(Value::Array(restored)),
    }
}

async fn rollback_change(
    client: &OvClient,
    snap_id: &str,
    change: &PendingMemoryChange,
) -> Result<Value, String> {
    let target = &change.target_uri;
    let before = change.before_content.as_deref();
    let result = match change.action {
        ChangeAction::Merge | ChangeAction::PromoteToIndex | ChangeAction::DemoteFromIndex => {
            match before {
                Some(content) => client.write_content(target, content).await,
                None => client.snapshot_restore_scoped(snap_id, Some(target)).await,
            }
        }
        ChangeAction::MoveFromInbox | ChangeAction::Archive => {
            let dest = change
                .destination_uri
                .as_deref()
                .ok_or("missing destination_uri")?;
            match (client.mv(dest, target).await, before) {
                (Err(_), Some(content)) => {
                    let response = client
                        .write_content(target, content)
                        .await
                        .map_err(|e| e.to_string())?;
                    match client.delete_fs(dest, false).await {
                        Err(e) if !is_not_found(&e) => Err(e),
                        _ => Ok(response),
                    }
                }
                (result, _) => result,
            }
        }
        ChangeAction::WikiCompile => {
            let dest = change
                .destination_uri
                .as_deref()
                .map(|uri| uri.trim_end_matches('/'))
                .filter(|uri| uri.starts_with("viking://"))
                .ok_or("missing or invalid Wiki destination URI")?;
            match client.snapshot_restore_scoped(snap_id, Some(dest)).await {
                Err(e)
                    if is_not_found(&e) || matches!(e, OvClientError::Api { status: 400, .. }) =>
                {
                    match client.delete_fs(dest, true).await {
                        Err(e) if is_not_found(&e) => Ok(json!({ "uri": dest })),
                        result => result,
                    }
                }
                result => result,
            }
        }
    };
    result.map_err(|e| e.to_string())
}

async fn read_text(client: &OvClient, uri: &str) -> Result<String, String> {
    let val = client.read_content(uri, "l2").await.map_err(|e| e.to_string())?;
    val.as_str()
        .or_else(|| val.get("content").and_then(Value::as_str))
        .or_else(|| val.get("text").and_then(Value::as_str))
        .map(str::to_owned)
        .ok_or_else(|| format!("Missing text content for {uri}"))
}

fn is_not_found(error: &OvClientError) -> bool {
    matches!(error, OvClientError::Api { status, code, .. }
        if *status == 404 || (code == "NOT_FOUND" && matches!(*status, 200..=299 | 400)))
}

async fn list_project_dirs(client: &OvClient, root: &str) -> Vec<String> {
    let Ok(value) = client.ls(root).await else {
        return Vec::new();
    };
    listing_entries(&value)
        .into_iter()
        .filter(|entry| is_directory(entry))
        .filter_map(|entry| entry.get("uri").and_then(Value::as_str))
        .filter(|uri| uri.starts_with(root) && *uri != root)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn listing_entries(value: &Value) -> Vec<&Value> {
    fn collect<'a>(value: &'a Value, entries: &mut Vec<&'a Value>) {
        match value {
            Value::Array(items) => {
                for item in items {
                    collect(item, entries);
                }
            }
            Value::Object(fields) => {
                if fields.contains_key("uri") {
                    entries.push(value);
                }
                for key in ["items", "entries", "children", "files", "results"] {
                    if let Some(items) = fields.get(key) {
                        collect(items, entries);
                    }
                }
            }
            Value::String(_) => entries.push(value),
            _ => {}
        }
    }

    let mut entries = Vec::new();
    collect(value, &mut entries);
    entries
}

fn is_directory(entry: &Value) -> bool {
    entry.get("isDir").and_then(Value::as_bool) == Some(true)
        || matches!(entry.get("type").and_then(Value::as_str), Some("dir" | "directory"))
}

fn file_uri(entry: &Value) -> Option<&str> {
    if is_directory(entry) {
        return None;
    }
    entry
        .get("uri")
        .and_then(Value::as_str)
        .or_else(|| entry.as_str())
}

fn extract_scored_uris(val: &Value) -> Vec<(String, f64)> {
    let mut slices: Vec<&[Value]> = Vec::new();
    if let Some(items) = val.as_array() {
        slices.push(items);
    } else {
        for key in ["items", "results", "entries", "memories", "resources", "skills"] {
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
