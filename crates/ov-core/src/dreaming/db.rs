use crate::platform::PlatformPaths;
use crate::policy::model::{
    ChangeAction, MemoryCategory, PendingMemoryChange, ReviewStatus, RiskLevel,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamingRun {
    pub id: String,
    #[serde(default)]
    pub profile_key: String,
    pub trigger_source: String,
    pub status: String,
    pub snapshot_id: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub summary_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallEvent {
    pub id: String,
    pub agent_id: String,
    pub event_type: String,
    pub query: Option<String>,
    pub target_uri: Option<String>,
    pub layer: Option<String>,
    pub tokens_used: i64,
    pub latency_ms: i64,
    pub blocked_by_privacy: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommittedTranscriptRow {
    pub seq: i64,
    pub session_key: String,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UriHitStat {
    pub uri: String,
    pub hit_count: u32,
    pub last_hit_at: i64,
    pub agents: Vec<String>,
}

/// Thread-safe (`Send + Sync`) SQLite wrapper for OpenViking Desktop local state.
pub struct DesktopDb {
    conn: Mutex<Connection>,
}

const CHANGE_COLUMNS: &str =
    "id, run_id, snapshot_id, action, risk_level, target_uri, destination_uri, category, base_hash,
     before_content, after_content, rationale, evidence_json, status, created_at";

const RUN_COLUMNS: &str =
    "id, trigger_source, status, snapshot_id, started_at, finished_at, summary_json, profile_key";

impl DesktopDb {
    pub fn open_default() -> Result<Self, String> {
        let paths = PlatformPaths::detect().ok_or("Cannot locate home directory")?;
        let db_path = paths.desktop_db();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        Self::open(&db_path)
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub(crate) fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "DesktopDb mutex poisoned".to_string())
    }

    fn init_schema(&self) -> Result<(), String> {
        let mut conn = self.lock_conn()?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(|e| e.to_string())?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let had_privacy_exclusions: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'privacy_exclusions')",
            [],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        tx.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS dreaming_runs (
                id TEXT PRIMARY KEY,
                profile_key TEXT NOT NULL DEFAULT '',
                trigger_source TEXT NOT NULL,
                status TEXT NOT NULL,
                snapshot_id TEXT,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                summary_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS pending_memory_changes (
                id TEXT PRIMARY KEY,
                profile_key TEXT NOT NULL DEFAULT '',
                run_id TEXT NOT NULL,
                snapshot_id TEXT,
                action TEXT NOT NULL,
                risk_level TEXT NOT NULL DEFAULT 'medium',
                target_uri TEXT NOT NULL,
                destination_uri TEXT,
                category TEXT NOT NULL,
                base_hash TEXT,
                before_content TEXT,
                after_content TEXT,
                rationale TEXT NOT NULL,
                evidence_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS recall_events (
                id TEXT PRIMARY KEY,
                profile_key TEXT NOT NULL DEFAULT '',
                agent_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                query TEXT,
                target_uri TEXT,
                layer TEXT,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                blocked_by_privacy INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pinned_uris (
                uri TEXT PRIMARY KEY,
                profile_key TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS scoped_pinned_uris (
                profile_key TEXT NOT NULL,
                uri TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (profile_key, uri)
            );

            CREATE TABLE IF NOT EXISTS recall_hits (
                event_id TEXT NOT NULL,
                memory_uri TEXT NOT NULL,
                score REAL NOT NULL,
                injected INTEGER NOT NULL,
                PRIMARY KEY (event_id, memory_uri)
            );
            CREATE TABLE IF NOT EXISTS capture_tasks (
                profile_key TEXT NOT NULL,
                task_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'accepted',
                PRIMARY KEY(profile_key,task_id)
            );
            CREATE TABLE IF NOT EXISTS capture_sessions (
                profile_key TEXT NOT NULL,
                session_key TEXT NOT NULL,
                agent TEXT NOT NULL,
                resource_uri TEXT NOT NULL,
                remote_id TEXT,
                peer_id TEXT,
                flush_requested INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL DEFAULT 'idle',
                claimed_at INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                PRIMARY KEY (profile_key, session_key)
            );
            CREATE TABLE IF NOT EXISTS capture_outbox (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_key TEXT NOT NULL,
                session_key TEXT NOT NULL,
                event_key TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'pending',
                UNIQUE(profile_key, session_key, event_key)
            );
            CREATE TABLE IF NOT EXISTS committed_transcript_evidence (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_key TEXT NOT NULL,
                session_key TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                committed_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS committed_transcript_evidence_profile_seq
                ON committed_transcript_evidence (profile_key, seq);
            CREATE TABLE IF NOT EXISTS privacy_exclusions (
                session_key TEXT NOT NULL,
                event_key TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (session_key, event_key)
            );
            CREATE TABLE IF NOT EXISTS kv_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())?;
        if !had_privacy_exclusions {
            // Legacy session hashes include the route and cannot be reversed. Honor their
            // skipped event keys across routes, even when the original route is unknown.
            tx.execute(
                "INSERT OR IGNORE INTO privacy_exclusions (session_key, event_key, created_at)
                 SELECT '', event_key, ?1 FROM capture_outbox WHERE state = 'skipped'",
                [Utc::now().timestamp()],
            ).map_err(|e| e.to_string())?;
        }
        for table in ["dreaming_runs", "pending_memory_changes", "recall_events", "pinned_uris"] {
            let has_profile: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = 'profile_key')",
                [table],
                |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if !has_profile {
                tx.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN profile_key TEXT NOT NULL DEFAULT ''"),
                    [],
                ).map_err(|e| e.to_string())?;
            }
        }
        tx.execute_batch(
            "INSERT OR IGNORE INTO scoped_pinned_uris (profile_key, uri, created_at)
             SELECT profile_key, uri, created_at FROM pinned_uris;
             CREATE INDEX IF NOT EXISTS recall_events_profile_created
                 ON recall_events (profile_key, created_at);
             CREATE INDEX IF NOT EXISTS dreaming_runs_profile_started
                 ON dreaming_runs (profile_key, started_at);
             CREATE INDEX IF NOT EXISTS pending_changes_profile_status
                 ON pending_memory_changes (profile_key, status);
             CREATE INDEX IF NOT EXISTS pending_changes_profile_run
                 ON pending_memory_changes (profile_key, run_id);
             CREATE INDEX IF NOT EXISTS capture_outbox_skipped_event
                 ON capture_outbox (session_key, event_key) WHERE state = 'skipped';",
        ).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn record_event(&self, evt: &RecallEvent) -> Result<(), String> {
        self.record_event_for_profile("", evt)
    }

    pub fn record_event_for_profile(&self, profile_key: &str, evt: &RecallEvent) -> Result<(), String> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO recall_events (id, agent_id, event_type, query, target_uri, layer, tokens_used, latency_ms, blocked_by_privacy, created_at, profile_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                evt.id,
                evt.agent_id,
                evt.event_type,
                evt.query,
                evt.target_uri,
                evt.layer,
                evt.tokens_used,
                evt.latency_ms,
                if evt.blocked_by_privacy { 1 } else { 0 },
                evt.created_at,
                profile_key,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn record_recall_hits(&self, event_id: &str, hits: &[(String, f64, bool)]) -> Result<(), String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (uri, score, injected) in hits {
            tx.execute("INSERT OR IGNORE INTO recall_hits(event_id,memory_uri,score,injected) VALUES (?1,?2,?3,?4)",
                params![event_id,uri,score,*injected]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn list_events_for_profile(
        &self,
        profile_key: &str,
        agent_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RecallEvent>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, event_type, query, target_uri, layer, tokens_used, latency_ms, blocked_by_privacy, created_at
             FROM recall_events WHERE profile_key = ?1 AND (?2 IS NULL OR agent_id = ?2)
             ORDER BY created_at DESC LIMIT ?3",
        ).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![profile_key, agent_filter, limit as i64], map_event_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn count_sessions_since(&self, profile_key: &str, since: i64) -> Result<i64, String> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT COUNT(*) FROM recall_events
             WHERE profile_key = ?1 AND created_at > ?2 AND blocked_by_privacy = 0
                   AND event_type IN ('hook_stop', 'capture')",
            params![profile_key, since],
            |row| row.get(0),
        ).map_err(|e| e.to_string())
    }

    pub fn hit_stats_by_uri_for_profile(&self, profile_key: &str) -> Result<HashMap<String, UriHitStat>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT uri, COUNT(*), MAX(created_at), GROUP_CONCAT(DISTINCT agent_id)
                 FROM (
                    SELECT h.memory_uri AS uri, e.created_at, e.agent_id
                    FROM recall_hits h JOIN recall_events e ON h.event_id=e.id
                    WHERE e.profile_key=?1 AND e.event_type='recall' AND e.blocked_by_privacy=0 AND h.injected=1
                    UNION ALL
                    SELECT target_uri AS uri, created_at, agent_id FROM recall_events e
                    WHERE profile_key=?1 AND target_uri IS NOT NULL AND event_type='recall' AND blocked_by_privacy=0
                    AND NOT EXISTS (SELECT 1 FROM recall_hits h WHERE h.event_id=e.id)
                 ) GROUP BY uri",
            )
            .map_err(|e| e.to_string())?;
        let mut map = HashMap::new();
        let rows = stmt
            .query_map([profile_key], |row| {
                let uri: String = row.get(0)?;
                let count: u32 = row.get(1)?;
                let last_hit_at: i64 = row.get(2)?;
                let agents_csv: Option<String> = row.get(3)?;
                let agents = agents_csv
                    .unwrap_or_default()
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                Ok(UriHitStat {
                    uri,
                    hit_count: count,
                    last_hit_at,
                    agents,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            let stat = r.map_err(|e| e.to_string())?;
            map.insert(stat.uri.clone(), stat);
        }
        Ok(map)
    }

    pub fn create_run_for_profile(&self, profile_key: &str, id: &str, trigger_source: &str) -> Result<DreamingRun, String> {
        let conn = self.lock_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO dreaming_runs (id, trigger_source, status, started_at, profile_key)
             VALUES (?1, ?2, 'running', ?3, ?4)",
            params![id, trigger_source, now, profile_key],
        )
        .map_err(|e| e.to_string())?;
        Ok(DreamingRun {
            id: id.to_string(),
            profile_key: profile_key.to_string(),
            trigger_source: trigger_source.to_string(),
            status: "running".to_string(),
            snapshot_id: None,
            started_at: now,
            finished_at: None,
            summary_json: "{}".to_string(),
        })
    }

    pub fn update_run_status_for_profile(
        &self,
        profile_key: &str,
        run_id: &str,
        status: &str,
        snapshot_id: Option<&str>,
        summary_json: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.lock_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE dreaming_runs
             SET status = ?1,
                 snapshot_id = COALESCE(?2, snapshot_id),
                 summary_json = COALESCE(?3, summary_json),
                 finished_at = ?4
             WHERE id = ?5 AND profile_key = ?6",
            params![status, snapshot_id, summary_json, now, run_id, profile_key],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_runs_for_profile(&self, profile_key: &str, limit: usize) -> Result<Vec<DreamingRun>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(&format!("SELECT {RUN_COLUMNS} FROM dreaming_runs
                WHERE profile_key = ?1 ORDER BY started_at DESC LIMIT ?2"))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![profile_key, limit as i64], map_run_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<DreamingRun>, String> {
        let conn = self.lock_conn()?;
        conn.query_row(
            &format!("SELECT {RUN_COLUMNS} FROM dreaming_runs WHERE id = ?1"),
            [run_id],
            map_run_row,
        ).optional().map_err(|e| e.to_string())
    }

    pub fn insert_pending_change_for_profile(&self, profile_key: &str, c: &PendingMemoryChange) -> Result<(), String> {
        let conn = self.lock_conn()?;
        let action = serde_json::to_string(&c.action)
            .unwrap()
            .trim_matches('"')
            .to_string();
        let risk = serde_json::to_string(&c.risk_level)
            .unwrap()
            .trim_matches('"')
            .to_string();
        let category = c.category.as_str();
        let status = c.status.as_str();
        conn.execute(
            "INSERT INTO pending_memory_changes
             (id, run_id, snapshot_id, action, risk_level, target_uri, destination_uri, category, base_hash, before_content, after_content, rationale, evidence_json, status, created_at, profile_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                c.id,
                c.run_id,
                c.snapshot_id,
                action,
                risk,
                c.target_uri,
                c.destination_uri,
                category,
                c.base_hash,
                c.before_content,
                c.after_content,
                c.rationale,
                c.evidence_json,
                status,
                c.created_at,
                profile_key,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_pending_changes_for_profile(
        &self,
        profile_key: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<PendingMemoryChange>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {CHANGE_COLUMNS} FROM pending_memory_changes
             WHERE profile_key = ?1 AND (?2 IS NULL OR status = ?2 OR
                   (?2 = 'conflict' AND status IN ('failed', 'partial_failed', 'partial_primary_failed')))
             ORDER BY created_at DESC",
        )).map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![profile_key, status_filter], map_change_row)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub(crate) fn list_applied_changes_for_run(
        &self,
        profile_key: &str,
        run_id: &str,
    ) -> Result<Vec<(PendingMemoryChange, bool)>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {CHANGE_COLUMNS} FROM pending_memory_changes
             WHERE profile_key = ?1 AND run_id = ?2
               AND status IN ('applied', 'partial_failed', 'partial_primary_failed')",
        )).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![profile_key, run_id], |row| {
                let status_str: String = row.get(13)?;
                let primary_only = status_str == "partial_primary_failed";
                Ok((map_change_row(row)?, primary_only))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
    }

    pub fn claim_accepted_changes_for_profile(
        &self,
        profile_key: &str,
    ) -> Result<Vec<PendingMemoryChange>, String> {
        self.claim_changes_for_profile(profile_key, None, ReviewStatus::Accepted)
    }

    pub fn claim_accepted_changes_by_ids_for_profile(
        &self,
        profile_key: &str,
        ids: &[String],
    ) -> Result<Vec<PendingMemoryChange>, String> {
        self.claim_changes_for_profile(profile_key, Some(ids), ReviewStatus::Accepted)
    }

    pub(crate) fn claim_auto_apply_changes_for_profile(
        &self,
        profile_key: &str,
        ids: &[String],
    ) -> Result<Vec<PendingMemoryChange>, String> {
        // Claim pending proposals atomically so a simultaneous manual apply cannot take them.
        self.claim_changes_for_profile(profile_key, Some(ids), ReviewStatus::Pending)
    }

    fn claim_changes_for_profile(
        &self,
        profile_key: &str,
        ids: Option<&[String]>,
        status: ReviewStatus,
    ) -> Result<Vec<PendingMemoryChange>, String> {
        let ids = ids.map(serde_json::to_string).transpose().map_err(|e| e.to_string())?;
        let applying = if status == ReviewStatus::Pending { "auto_applying" } else { "applying" };
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let mut changes = {
            let mut stmt = tx.prepare(&format!(
                "UPDATE pending_memory_changes SET status = ?4
                 WHERE status = ?3 AND profile_key = ?1 AND profile_key != ''
                   AND (?2 IS NULL OR id IN (SELECT value FROM json_each(?2)))
                 RETURNING {CHANGE_COLUMNS}",
            )).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![profile_key, ids, status.as_str(), applying], map_change_row)
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
        };
        if !changes.is_empty() {
            tx.execute(
                "UPDATE dreaming_runs SET status = 'applying'
                 WHERE profile_key = ?1 AND id IN (
                     SELECT run_id FROM pending_memory_changes
                     WHERE profile_key = ?1 AND status IN ('applying', 'auto_applying')
                 )",
                [profile_key],
            ).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        changes.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        Ok(changes)
    }

    pub(crate) fn release_claim_for_profile(
        &self,
        profile_key: &str,
        changes: &[PendingMemoryChange],
        retry_status: ReviewStatus,
    ) -> Result<(), String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut runs = HashSet::new();
        for change in changes {
            tx.execute(
                "UPDATE pending_memory_changes SET status = ?3
                 WHERE profile_key = ?1 AND id = ?2 AND status IN ('applying', 'auto_applying')",
                params![profile_key, change.id, retry_status.as_str()],
            ).map_err(|e| e.to_string())?;
            runs.insert(change.run_id.as_str());
        }
        for run_id in runs {
            refresh_run_outcomes(&tx, profile_key, run_id).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn finalize_dreaming_run(
        &self,
        profile_key: &str,
        run_id: &str,
        summary: &str,
    ) -> Result<(DreamingRun, Vec<PendingMemoryChange>), String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        refresh_run_outcomes(&tx, profile_key, run_id).map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE dreaming_runs SET summary_json = ?3 WHERE id = ?1 AND profile_key = ?2",
            params![run_id, profile_key, summary],
        ).map_err(|e| e.to_string())?;
        let run = tx.query_row(
            &format!("SELECT {RUN_COLUMNS} FROM dreaming_runs WHERE id = ?1 AND profile_key = ?2"),
            params![run_id, profile_key],
            map_run_row,
        ).map_err(|e| e.to_string())?;
        let changes = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {CHANGE_COLUMNS} FROM pending_memory_changes
                 WHERE run_id = ?1 AND profile_key = ?2 ORDER BY rowid",
            )).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![run_id, profile_key], map_change_row)
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        tx.commit().map_err(|e| e.to_string())?;
        Ok((run, changes))
    }

    /// Legacy entry point for trusted local callers that already hold a change ID.
    pub fn update_change_decision(
        &self,
        id: &str,
        status: ReviewStatus,
        edited_after_content: Option<&str>,
    ) -> Result<(), String> {
        self.write_change_decision(None, id, status, edited_after_content)
    }

    pub fn update_change_decision_for_profile(
        &self,
        profile_key: &str,
        id: &str,
        status: ReviewStatus,
        edited_after_content: Option<&str>,
    ) -> Result<(), String> {
        self.write_change_decision(Some(profile_key), id, status, edited_after_content)
    }

    fn write_change_decision(
        &self,
        profile_key: Option<&str>,
        id: &str,
        status: ReviewStatus,
        edited_after_content: Option<&str>,
    ) -> Result<(), String> {
        if !matches!(status, ReviewStatus::Pending | ReviewStatus::Accepted | ReviewStatus::Rejected) {
            return Err("Invalid review decision".into());
        }
        let conn = self.lock_conn()?;
        let updated = conn.execute(
            "UPDATE pending_memory_changes
             SET status = ?1, after_content = COALESCE(?2, after_content)
             WHERE id = ?3 AND (?4 IS NULL OR profile_key = ?4)
                   AND status IN ('pending', 'accepted', 'rejected')",
            params![status.as_str(), edited_after_content, id, profile_key],
        ).map_err(|e| e.to_string())?;
        if updated == 0 {
            return Err(format!("Change {id} is missing or no longer open for review"));
        }
        Ok(())
    }

    pub(crate) fn finish_claimed_change_for_profile(
        &self,
        profile_key: &str,
        change: &PendingMemoryChange,
        snapshot_id: Option<&str>,
        status: &str,
    ) -> Result<(), String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let updated = tx.execute(
            "UPDATE pending_memory_changes SET status = ?1, snapshot_id = ?2
             WHERE id = ?3 AND profile_key = ?4 AND status IN ('applying', 'auto_applying')",
            params![status, snapshot_id, change.id, profile_key],
        ).map_err(|e| e.to_string())?;
        if updated != 1 {
            return Err(format!("Lost claim for change {}", change.id));
        }
        refresh_run_outcomes(&tx, profile_key, &change.run_id).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn pin_uri_for_profile(&self, profile_key: &str, uri: &str) -> Result<(), String> {
        let conn = self.lock_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO scoped_pinned_uris (profile_key, uri, created_at) VALUES (?1, ?2, ?3)",
            params![profile_key, uri, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn is_pinned_for_profile(&self, profile_key: &str, uri: &str) -> Result<bool, String> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM scoped_pinned_uris
             WHERE uri = ?2 AND (profile_key = ?1 OR profile_key = ''))",
            params![profile_key, uri],
            |row| row.get(0),
        ).map_err(|e| e.to_string())
    }

    pub fn get_kv(&self, key: &str) -> Result<Option<String>, String> {
        self.lock_conn()?.query_row(
            "SELECT value FROM kv_state WHERE key = ?1",
            [key],
            |row| row.get(0),
        ).optional().map_err(|e| e.to_string())
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<(), String> {
        self.lock_conn()?.execute(
            "INSERT INTO kv_state (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn recent_committed_transcripts_for_profile(
        &self,
        profile_key: &str,
        since_seq: i64,
        limit: usize,
    ) -> Result<Vec<CommittedTranscriptRow>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT seq, session_key, role, content
                 FROM committed_transcript_evidence
                 WHERE profile_key = ?1 AND seq > ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![profile_key, since_seq, limit as i64], |r| {
                Ok(CommittedTranscriptRow {
                    seq: r.get(0)?,
                    session_key: r.get(1)?,
                    role: r.get(2)?,
                    content: r.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn max_committed_outbox_seq_for_profile(&self, profile_key: &str) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM committed_transcript_evidence WHERE profile_key = ?1",
            params![profile_key],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    }

    /// Recover claims left stuck in `applying` after an unexpected app exit.
    pub fn recover_stale_claims(&self) -> Result<usize, String> {
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let runs: Vec<(String, String)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT DISTINCT profile_key, run_id FROM pending_memory_changes
                     WHERE status IN ('applying', 'auto_applying')",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        let recovered = tx
            .execute(
                "UPDATE pending_memory_changes
                 SET status = CASE status WHEN 'auto_applying' THEN 'pending' ELSE 'accepted' END
                 WHERE status IN ('applying', 'auto_applying')",
                [],
            )
            .map_err(|e| e.to_string())?;
        for (profile_key, run_id) in runs {
            refresh_run_outcomes(&tx, &profile_key, &run_id).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(recovered)
    }

    pub fn claim_schedule_slot(&self, profile_key: &str, timestamp: i64) -> Result<bool, String> {
        let changed = self.lock_conn()?.execute(
            "INSERT INTO kv_state (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value
             WHERE CAST(kv_state.value AS INTEGER) < ?2",
            params![format!("dreaming.schedule.{profile_key}"), timestamp],
        ).map_err(|e| e.to_string())?;
        Ok(changed == 1)
    }
}

fn refresh_run_outcomes(conn: &Connection, profile_key: &str, run_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "WITH outcomes AS (
            SELECT COUNT(*) FILTER (WHERE status IN ('applying', 'auto_applying')) AS applying,
                   COUNT(*) FILTER (WHERE status IN ('pending', 'accepted')) AS review,
                   COUNT(*) FILTER (WHERE status IN ('failed', 'partial_failed', 'partial_primary_failed')) AS failed,
                   COUNT(*) FILTER (WHERE status = 'conflict') AS conflicts,
                   COUNT(*) FILTER (WHERE status = 'applied') AS applied,
                   (SELECT snapshot_id FROM pending_memory_changes
                    WHERE run_id = ?1 AND profile_key = ?2
                      AND status IN ('applied', 'partial_failed', 'partial_primary_failed')
                      AND snapshot_id IS NOT NULL
                    ORDER BY rowid ASC LIMIT 1) AS snapshot
            FROM pending_memory_changes WHERE run_id = ?1 AND profile_key = ?2
         )
         UPDATE dreaming_runs SET
            status = (SELECT CASE WHEN applying > 0 THEN 'applying'
                                  WHEN failed > 0 OR (conflicts > 0 AND applied > 0) THEN 'partial_failure'
                                  WHEN conflicts > 0 THEN 'conflict'
                                  WHEN review > 0 THEN 'ready_for_review'
                                  WHEN applied > 0 THEN 'applied'
                                  ELSE 'ready_for_review' END FROM outcomes),
            snapshot_id = (SELECT snapshot FROM outcomes),
            finished_at = ?3
         WHERE id = ?1 AND profile_key = ?2",
        params![run_id, profile_key, Utc::now().timestamp()],
    )?;
    Ok(())
}

fn map_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DreamingRun> {
    Ok(DreamingRun {
        id: row.get(0)?,
        trigger_source: row.get(1)?,
        status: row.get(2)?,
        snapshot_id: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
        summary_json: row.get(6)?,
        profile_key: row.get(7)?,
    })
}

fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecallEvent> {
    let blocked: i64 = row.get(8)?;
    Ok(RecallEvent {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        event_type: row.get(2)?,
        query: row.get(3)?,
        target_uri: row.get(4)?,
        layer: row.get(5)?,
        tokens_used: row.get(6)?,
        latency_ms: row.get(7)?,
        blocked_by_privacy: blocked != 0,
        created_at: row.get(9)?,
    })
}

fn map_change_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingMemoryChange> {
    let action_str: String = row.get(3)?;
    let risk_str: String = row.get(4)?;
    let cat_str: String = row.get(7)?;
    let status_str: String = row.get(13)?;

    let action: ChangeAction = serde_json::from_str(&format!("\"{}\"", action_str))
        .unwrap_or(ChangeAction::Merge);
    let risk_level: RiskLevel = serde_json::from_str(&format!("\"{}\"", risk_str))
        .unwrap_or(RiskLevel::Medium);
    let category: MemoryCategory = serde_json::from_str(&format!("\"{}\"", cat_str))
        .unwrap_or(MemoryCategory::Patterns);
    let status = match status_str.as_str() {
        "applying" | "auto_applying" => ReviewStatus::Accepted,
        "failed" | "partial_failed" | "partial_primary_failed" => ReviewStatus::Conflict,
        _ => serde_json::from_str(&format!("\"{}\"", status_str))
            .unwrap_or(ReviewStatus::Pending),
    };

    Ok(PendingMemoryChange {
        id: row.get(0)?,
        run_id: row.get(1)?,
        snapshot_id: row.get(2)?,
        action,
        risk_level,
        target_uri: row.get(5)?,
        destination_uri: row.get(6)?,
        category,
        base_hash: row.get(8)?,
        before_content: row.get(9)?,
        after_content: row.get(10)?,
        rationale: row.get(11)?,
        evidence_json: row.get(12)?,
        status,
        created_at: row.get(14)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn test_desktop_db_is_send_sync() {
        assert_send_sync::<DesktopDb>();
    }

    fn archive_change(id: &str, run_id: &str) -> PendingMemoryChange {
        PendingMemoryChange {
            id: id.to_string(),
            run_id: run_id.to_string(),
            snapshot_id: None,
            created_at: Utc::now().timestamp(),
            action: ChangeAction::Archive,
            risk_level: RiskLevel::Low,
            target_uri: "viking://user/memories/entities/old.md".to_string(),
            destination_uri: Some("viking://~/archive/old.md".to_string()),
            category: MemoryCategory::Entities,
            base_hash: Some("abc123".to_string()),
            before_content: Some("old entity".to_string()),
            after_content: None,
            rationale: "Stale for 50 days".to_string(),
            evidence_json: "{}".to_string(),
            status: ReviewStatus::Pending,
        }
    }

    #[test]
    fn test_sqlite_dreaming_lifecycle() {
        let db = DesktopDb::open_in_memory().unwrap();
        let run = db.create_run_for_profile("a", "run-1", "manual").unwrap();
        assert_eq!(run.status, "running");
        let change = archive_change("chg-1", &run.id);
        db.insert_pending_change_for_profile("a", &change).unwrap();
        let listed = db.list_pending_changes_for_profile("a", Some("pending")).unwrap();
        assert_eq!(listed.len(), 1);

        db.update_change_decision_for_profile("a", "chg-1", ReviewStatus::Accepted, None)
            .unwrap();
        let claimed = db.claim_accepted_changes_for_profile("a").unwrap();
        assert_eq!(claimed.len(), 1);
        assert!(db.update_change_decision("chg-1", ReviewStatus::Accepted, None).is_err());
        db.release_claim_for_profile("a", &claimed, ReviewStatus::Accepted).unwrap();
        let claimed = db.claim_accepted_changes_for_profile("a").unwrap();
        db.finish_claimed_change_for_profile("a", &claimed[0], None, "conflict")
            .unwrap();
        let resolved = db.get_run(&run.id).unwrap().unwrap();
        assert_eq!(resolved.status, "conflict");
        assert_eq!(resolved.snapshot_id, None);
    }

    #[test]
    fn profiles_isolate_runs_changes_pins_and_events() {
        let db = DesktopDb::open_in_memory().unwrap();
        let run = db.create_run_for_profile("a", "run-a", "manual").unwrap();
        let mut change = archive_change("chg-a", &run.id);
        change.status = ReviewStatus::Accepted;
        db.insert_pending_change_for_profile("a", &change).unwrap();
        db.pin_uri_for_profile("a", &change.target_uri).unwrap();

        assert!(db.list_runs_for_profile("b", 5).unwrap().is_empty());
        assert!(db.list_pending_changes_for_profile("b", None).unwrap().is_empty());
        assert!(db.update_change_decision_for_profile("b", &change.id, ReviewStatus::Rejected, None).is_err());
        assert!(db.claim_accepted_changes_for_profile("b").unwrap().is_empty());
        assert!(!db.is_pinned_for_profile("b", &change.target_uri).unwrap());
        db.pin_uri_for_profile("b", &change.target_uri).unwrap();
        assert!(db.is_pinned_for_profile("a", &change.target_uri).unwrap());
        assert!(db.is_pinned_for_profile("b", &change.target_uri).unwrap());

        let mut event = RecallEvent {
            id: "recall-a".into(),
            agent_id: "agent".into(),
            event_type: "recall".into(),
            query: None,
            target_uri: Some(change.target_uri.clone()),
            layer: None,
            tokens_used: 0,
            latency_ms: 0,
            blocked_by_privacy: false,
            created_at: 10,
        };
        db.record_event_for_profile("a", &event).unwrap();
        event.id = "legacy-recall".into();
        db.record_event(&event).unwrap();
        assert_eq!(db.list_events_for_profile("a", Some("agent"), 10).unwrap().len(), 1);
        assert!(db.list_events_for_profile("b", None, 10).unwrap().is_empty());
        assert_eq!(db.hit_stats_by_uri_for_profile("a").unwrap()[&change.target_uri].hit_count, 1);
        assert!(db.hit_stats_by_uri_for_profile("b").unwrap().is_empty());

        event.id = "hook-a".into();
        event.event_type = "hook_stop".into();
        db.record_event_for_profile("a", &event).unwrap();
        event.id = "capture-a".into();
        event.event_type = "capture".into();
        event.blocked_by_privacy = true;
        db.record_event_for_profile("a", &event).unwrap();
        assert_eq!(db.count_sessions_since("a", 0).unwrap(), 1);
        assert_eq!(db.count_sessions_since("b", 0).unwrap(), 0);
        assert_eq!(db.count_sessions_since("a", 10).unwrap(), 0);
    }

    #[test]
    fn accepted_changes_are_claimed_once_across_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.db");
        let db = DesktopDb::open(&path).unwrap();
        let other = DesktopDb::open(&path).unwrap();
        let mut change = archive_change("claim-a", "run-a");
        change.status = ReviewStatus::Accepted;
        db.insert_pending_change_for_profile("a", &change).unwrap();
        change.id = "legacy-change".into();
        db.insert_pending_change_for_profile("", &change).unwrap();

        let gate = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let thread = scope.spawn(|| {
                gate.wait();
                other.claim_accepted_changes_for_profile("a").unwrap()
            });
            gate.wait();
            let first_claim = db.claim_accepted_changes_for_profile("a").unwrap();
            let second_claim = thread.join().unwrap();
            assert_eq!(first_claim.len() + second_claim.len(), 1);
        });
        assert!(db.claim_accepted_changes_for_profile("").unwrap().is_empty());
        assert_eq!(db.list_pending_changes_for_profile("", Some("accepted")).unwrap().len(), 1);
    }

    #[test]
    fn claims_by_id_are_profile_scoped_and_auto_claims_recover_as_pending() {
        let db = DesktopDb::open_in_memory().unwrap();
        db.create_run_for_profile("a", "current", "manual").unwrap();
        let mut low = archive_change("low", "current");
        low.status = ReviewStatus::Accepted;
        let mut high = archive_change("older-high", "previous");
        high.risk_level = RiskLevel::High;
        high.status = ReviewStatus::Accepted;
        let other = PendingMemoryChange { id: "other".into(), ..low.clone() };
        db.insert_pending_change_for_profile("a", &low).unwrap();
        db.insert_pending_change_for_profile("a", &high).unwrap();
        db.insert_pending_change_for_profile("b", &other).unwrap();

        assert!(db.claim_accepted_changes_by_ids_for_profile("a", &[]).unwrap().is_empty());
        let claimed = db.claim_accepted_changes_by_ids_for_profile("a", &[low.id.clone(), other.id]).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, low.id);
        db.release_claim_for_profile("a", &claimed, ReviewStatus::Pending).unwrap();
        let claimed = db.claim_auto_apply_changes_for_profile("a", &[low.id.clone()]).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(db.get_run("current").unwrap().unwrap().status, "applying");
        assert_eq!(db.recover_stale_claims().unwrap(), 1);
        assert_eq!(db.list_pending_changes_for_profile("a", Some("pending")).unwrap()[0].id, low.id);
        assert_eq!(db.list_pending_changes_for_profile("a", Some("accepted")).unwrap()[0].id, high.id);
    }

    #[test]
    fn legacy_schema_migrates_and_schedule_claim_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("desktop.db");
        let legacy = Connection::open(&path).unwrap();
        legacy.execute_batch(
            "CREATE TABLE dreaming_runs (id TEXT PRIMARY KEY, started_at INTEGER);
             CREATE TABLE pending_memory_changes (id TEXT PRIMARY KEY, run_id TEXT, status TEXT);
             CREATE TABLE recall_events (id TEXT PRIMARY KEY, created_at INTEGER);
             CREATE TABLE pinned_uris (uri TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
             INSERT INTO pinned_uris VALUES ('viking://legacy', 1);",
        ).unwrap();
        drop(legacy);

        let db = DesktopDb::open(&path).unwrap();
        assert!(db.is_pinned_for_profile("", "viking://legacy").unwrap());
        assert!(db.is_pinned_for_profile("a", "viking://legacy").unwrap());
        assert!(db.claim_schedule_slot("a", 100).unwrap());
        let conn = db.lock_conn().unwrap();
        conn.execute_batch(
            "DROP TABLE privacy_exclusions;
             INSERT INTO capture_outbox (profile_key, session_key, event_key, role, content, state)
             VALUES ('old-profile', 'old-route-hash', 'private-event', 'user', '', 'skipped');",
        ).unwrap();
        drop(conn);
        drop(db);

        let db = DesktopDb::open(&path).unwrap();
        assert!(db.is_privacy_excluded("new-route-hash", "private-event").unwrap());
        assert!(!db.claim_schedule_slot("a", 100).unwrap());
        assert!(!db.claim_schedule_slot("a", 99).unwrap());
        assert!(db.claim_schedule_slot("b", 100).unwrap());
        assert!(db.claim_schedule_slot("a", 101).unwrap());
        db.set_kv("example", "value").unwrap();
        assert_eq!(db.get_kv("example").unwrap().as_deref(), Some("value"));
    }

    #[test]
    fn recent_committed_transcripts_are_profile_scoped_and_paginated() {
        let db = DesktopDb::open_in_memory().unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO committed_transcript_evidence (profile_key, session_key, role, content, committed_at) VALUES
                 ('p1', 'sess-1', 'user', 'We switched to Rust 1.85', 100),
                 ('p2', 'sess-2', 'user', 'Other profile message', 100),
                 ('p1', 'sess-1', 'assistant', 'Confirmed Cargo.toml uses 1.85', 100)",
                [],
            )
            .unwrap();
        }
        let rows = db.recent_committed_transcripts_for_profile("p1", 0, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].role, "user");
        assert_eq!(rows[1].role, "assistant");
        let page = db.recent_committed_transcripts_for_profile("p1", rows[0].seq, 1).unwrap();
        assert_eq!(page[0].seq, rows[1].seq);
        let max_seq = db.max_committed_outbox_seq_for_profile("p1").unwrap();
        assert_eq!(max_seq, rows.last().unwrap().seq);
        let next = db.recent_committed_transcripts_for_profile("p1", max_seq, 10).unwrap();
        assert!(next.is_empty());
    }
}
