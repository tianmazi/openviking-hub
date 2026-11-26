use crate::platform::PlatformPaths;
use crate::policy::model::{
    ChangeAction, MemoryCategory, PendingMemoryChange, ReviewStatus, RiskLevel,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamingRun {
    pub id: String,
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

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|_| "DesktopDb mutex poisoned".to_string())
    }

    fn init_schema(&self) -> Result<(), String> {
        let conn = self.lock_conn()?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS dreaming_runs (
                id TEXT PRIMARY KEY,
                trigger_source TEXT NOT NULL,
                status TEXT NOT NULL,
                snapshot_id TEXT,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                summary_json TEXT NOT NULL DEFAULT '{}'
            );

            CREATE TABLE IF NOT EXISTS pending_memory_changes (
                id TEXT PRIMARY KEY,
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
                created_at INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|e| e.to_string())
    }

    pub fn record_event(&self, evt: &RecallEvent) -> Result<(), String> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO recall_events (id, agent_id, event_type, query, target_uri, layer, tokens_used, latency_ms, blocked_by_privacy, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_events(
        &self,
        agent_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RecallEvent>, String> {
        let conn = self.lock_conn()?;
        let mut results = Vec::new();
        if let Some(agent) = agent_filter {
            let mut stmt = conn
                .prepare(
                    "SELECT id, agent_id, event_type, query, target_uri, layer, tokens_used, latency_ms, blocked_by_privacy, created_at
                     FROM recall_events WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![agent, limit as i64], map_event_row)
                .map_err(|e| e.to_string())?;
            for r in rows {
                results.push(r.map_err(|e| e.to_string())?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, agent_id, event_type, query, target_uri, layer, tokens_used, latency_ms, blocked_by_privacy, created_at
                     FROM recall_events ORDER BY created_at DESC LIMIT ?1",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![limit as i64], map_event_row)
                .map_err(|e| e.to_string())?;
            for r in rows {
                results.push(r.map_err(|e| e.to_string())?);
            }
        }
        Ok(results)
    }

    pub fn hit_stats_by_uri(&self) -> Result<HashMap<String, UriHitStat>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT target_uri, COUNT(*), MAX(created_at), GROUP_CONCAT(DISTINCT agent_id)
                 FROM recall_events WHERE target_uri IS NOT NULL AND event_type = 'recall'
                 GROUP BY target_uri",
            )
            .map_err(|e| e.to_string())?;
        let mut map = HashMap::new();
        let rows = stmt
            .query_map([], |row| {
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

    pub fn create_run(&self, id: &str, trigger_source: &str) -> Result<DreamingRun, String> {
        let conn = self.lock_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO dreaming_runs (id, trigger_source, status, started_at, summary_json)
             VALUES (?1, ?2, 'running', ?3, '{}')",
            params![id, trigger_source, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(DreamingRun {
            id: id.to_string(),
            trigger_source: trigger_source.to_string(),
            status: "running".to_string(),
            snapshot_id: None,
            started_at: now,
            finished_at: None,
            summary_json: "{}".to_string(),
        })
    }

    pub fn update_run_status(
        &self,
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
             WHERE id = ?5",
            params![status, snapshot_id, summary_json, now, run_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<DreamingRun>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, trigger_source, status, snapshot_id, started_at, finished_at, summary_json
                 FROM dreaming_runs ORDER BY started_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(DreamingRun {
                    id: row.get(0)?,
                    trigger_source: row.get(1)?,
                    status: row.get(2)?,
                    snapshot_id: row.get(3)?,
                    started_at: row.get(4)?,
                    finished_at: row.get(5)?,
                    summary_json: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<DreamingRun>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, trigger_source, status, snapshot_id, started_at, finished_at, summary_json
                 FROM dreaming_runs WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![run_id], |row| {
                Ok(DreamingRun {
                    id: row.get(0)?,
                    trigger_source: row.get(1)?,
                    status: row.get(2)?,
                    snapshot_id: row.get(3)?,
                    started_at: row.get(4)?,
                    finished_at: row.get(5)?,
                    summary_json: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?;
        if let Some(r) = rows.next() {
            Ok(Some(r.map_err(|e| e.to_string())?))
        } else {
            Ok(None)
        }
    }

    pub fn insert_pending_change(&self, c: &PendingMemoryChange) -> Result<(), String> {
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
             (id, run_id, snapshot_id, action, risk_level, target_uri, destination_uri, category, base_hash, before_content, after_content, rationale, evidence_json, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                c.created_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_pending_changes(
        &self,
        status_filter: Option<&str>,
    ) -> Result<Vec<PendingMemoryChange>, String> {
        let conn = self.lock_conn()?;
        let sql = if status_filter.is_some() {
            "SELECT id, run_id, snapshot_id, action, risk_level, target_uri, destination_uri, category, base_hash, before_content, after_content, rationale, evidence_json, status, created_at
             FROM pending_memory_changes WHERE status = ?1 ORDER BY created_at DESC"
        } else {
            "SELECT id, run_id, snapshot_id, action, risk_level, target_uri, destination_uri, category, base_hash, before_content, after_content, rationale, evidence_json, status, created_at
             FROM pending_memory_changes ORDER BY created_at DESC"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        if let Some(st) = status_filter {
            let rows = stmt
                .query_map(params![st], map_change_row)
                .map_err(|e| e.to_string())?;
            for r in rows {
                out.push(r.map_err(|e| e.to_string())?);
            }
        } else {
            let rows = stmt
                .query_map([], map_change_row)
                .map_err(|e| e.to_string())?;
            for r in rows {
                out.push(r.map_err(|e| e.to_string())?);
            }
        }
        Ok(out)
    }

    pub fn update_change_decision(
        &self,
        id: &str,
        status: ReviewStatus,
        edited_after_content: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.lock_conn()?;
        if let Some(after) = edited_after_content {
            conn.execute(
                "UPDATE pending_memory_changes SET status = ?1, after_content = ?2 WHERE id = ?3",
                params![status.as_str(), after, id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE pending_memory_changes SET status = ?1 WHERE id = ?2",
                params![status.as_str(), id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn mark_change_applied(
        &self,
        id: &str,
        snapshot_id: Option<&str>,
        status: ReviewStatus,
    ) -> Result<(), String> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE pending_memory_changes SET status = ?1, snapshot_id = ?2 WHERE id = ?3",
            params![status.as_str(), snapshot_id, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn pin_uri(&self, uri: &str) -> Result<(), String> {
        let conn = self.lock_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO pinned_uris (uri, created_at) VALUES (?1, ?2)",
            params![uri, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn is_pinned(&self, uri: &str) -> Result<bool, String> {
        let conn = self.lock_conn()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pinned_uris WHERE uri = ?1",
                params![uri],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    }
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
    let status: ReviewStatus = serde_json::from_str(&format!("\"{}\"", status_str))
        .unwrap_or(ReviewStatus::Pending);

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

    #[test]
    fn test_sqlite_dreaming_lifecycle() {
        let db = DesktopDb::open_in_memory().unwrap();
        let run = db.create_run("run-1", "manual").unwrap();
        assert_eq!(run.status, "running");

        let change = PendingMemoryChange {
            id: "chg-1".to_string(),
            run_id: "run-1".to_string(),
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
        };
        db.insert_pending_change(&change).unwrap();
        let listed = db.list_pending_changes(Some("pending")).unwrap();
        assert_eq!(listed.len(), 1);

        db.update_change_decision("chg-1", ReviewStatus::Accepted, None)
            .unwrap();
        let accepted = db.list_pending_changes(Some("accepted")).unwrap();
        assert_eq!(accepted.len(), 1);

        db.pin_uri("viking://user/memories/entities/old.md").unwrap();
        assert!(db.is_pinned("viking://user/memories/entities/old.md").unwrap());
    }
}
