use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyConfig {
    pub schema_version: u32,
    pub global: GlobalPolicy,
    pub routing: RoutingPolicy,
    pub budget: BudgetPolicy,
    pub lifecycle: LifecyclePolicy,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            global: GlobalPolicy::default(),
            routing: RoutingPolicy::default(),
            budget: BudgetPolicy::default(),
            lifecycle: LifecyclePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalPolicy {
    pub capture_enabled: bool,
    pub active_profile: String,
    pub default_visibility: VisibilityScope,
}

impl Default for GlobalPolicy {
    fn default() -> Self {
        Self {
            capture_enabled: true,
            active_profile: "default".to_string(),
            default_visibility: VisibilityScope::Private,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityScope {
    /// `viking://resources/`
    Shared,
    /// `viking://~/resources/`
    Private,
}

impl VisibilityScope {
    pub fn resources_prefix(self) -> &'static str {
        match self {
            VisibilityScope::Shared => "viking://resources",
            VisibilityScope::Private => "viking://~/resources",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingPolicy {
    pub auto_map_git_repo: bool,
    pub respect_project_policy_file: bool,
    pub isolate_agent_namespaces: bool,
    pub inbox_uri: String,
    pub protected_rule_globs: Vec<String>,
    pub workspace_mappings: Vec<WorkspaceMapping>,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            auto_map_git_repo: true,
            respect_project_policy_file: true,
            isolate_agent_namespaces: true,
            inbox_uri: "viking://~/resources/inbox/".to_string(),
            protected_rule_globs: vec![
                "CLAUDE.md".to_string(),
                "**/CLAUDE.md".to_string(),
                "AGENTS.md".to_string(),
                "**/AGENTS.md".to_string(),
                ".cursorrules".to_string(),
                ".agents/rules/**".to_string(),
            ],
            workspace_mappings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMapping {
    pub local_path: PathBuf,
    pub resource_uri: String,
    pub visibility: VisibilityScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetPolicy {
    pub startup_max_lines: usize,
    pub startup_max_bytes: usize,
    pub default_recall_layer: ContextLayer,
    pub auto_expand_l1_threshold: f32,
    pub max_recall_tokens: usize,
    pub category_quotas: HashMap<MemoryCategory, usize>,
    pub agent_overrides: HashMap<AgentId, AgentBudgetOverride>,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        let mut category_quotas = HashMap::new();
        category_quotas.insert(MemoryCategory::Profile, 200);
        category_quotas.insert(MemoryCategory::Preferences, 500);
        category_quotas.insert(MemoryCategory::Entities, 500);
        category_quotas.insert(MemoryCategory::Events, 300);
        category_quotas.insert(MemoryCategory::Cases, 300);
        category_quotas.insert(MemoryCategory::Patterns, 200);

        let mut agent_overrides = HashMap::new();
        agent_overrides.insert(
            AgentId::ClaudeCode,
            AgentBudgetOverride {
                startup_max_lines: None,
                startup_max_bytes: Some(9500),
                default_recall_layer: Some(ContextLayer::L0),
                max_recall_tokens: Some(2500),
            },
        );
        agent_overrides.insert(
            AgentId::Codex,
            AgentBudgetOverride {
                startup_max_lines: None,
                startup_max_bytes: None,
                default_recall_layer: Some(ContextLayer::L0),
                max_recall_tokens: Some(2000),
            },
        );
        agent_overrides.insert(
            AgentId::Pi,
            AgentBudgetOverride {
                startup_max_lines: None,
                startup_max_bytes: None,
                default_recall_layer: Some(ContextLayer::L1),
                max_recall_tokens: Some(3000),
            },
        );
        agent_overrides.insert(
            AgentId::Antigravity,
            AgentBudgetOverride {
                startup_max_lines: None,
                startup_max_bytes: None,
                default_recall_layer: Some(ContextLayer::L1),
                max_recall_tokens: Some(4000),
            },
        );

        Self {
            startup_max_lines: 200,
            startup_max_bytes: 24576,
            default_recall_layer: ContextLayer::L0,
            auto_expand_l1_threshold: 0.80,
            max_recall_tokens: 2000,
            category_quotas,
            agent_overrides,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ContextLayer {
    L0,
    L1,
    L2,
}

impl ContextLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextLayer::L0 => "l0",
            ContextLayer::L1 => "l1",
            ContextLayer::L2 => "l2",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Profile,
    Preferences,
    Entities,
    Events,
    Cases,
    Patterns,
}

impl MemoryCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryCategory::Profile => "profile",
            MemoryCategory::Preferences => "preferences",
            MemoryCategory::Entities => "entities",
            MemoryCategory::Events => "events",
            MemoryCategory::Cases => "cases",
            MemoryCategory::Patterns => "patterns",
        }
    }

    pub fn from_uri(uri: &str) -> Self {
        let lower = uri.to_lowercase();
        if lower.contains("/profile") {
            MemoryCategory::Profile
        } else if lower.contains("/preferences") {
            MemoryCategory::Preferences
        } else if lower.contains("/entities") {
            MemoryCategory::Entities
        } else if lower.contains("/events") {
            MemoryCategory::Events
        } else if lower.contains("/cases") {
            MemoryCategory::Cases
        } else {
            MemoryCategory::Patterns
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    ClaudeCode,
    Codex,
    Pi,
    Antigravity,
}

impl AgentId {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "claude-code",
            AgentId::Codex => "codex",
            AgentId::Pi => "pi",
            AgentId::Antigravity => "antigravity",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Some(AgentId::ClaudeCode),
            "codex" => Some(AgentId::Codex),
            "pi" => Some(AgentId::Pi),
            "antigravity" | "agy" => Some(AgentId::Antigravity),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentBudgetOverride {
    pub startup_max_lines: Option<usize>,
    pub startup_max_bytes: Option<usize>,
    pub default_recall_layer: Option<ContextLayer>,
    pub max_recall_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectiveBudget {
    pub startup_max_lines: usize,
    pub startup_max_bytes: usize,
    pub default_recall_layer: ContextLayer,
    pub max_recall_tokens: usize,
    pub auto_expand_l1_threshold: f32,
}

impl BudgetPolicy {
    pub fn resolve_for(&self, agent: AgentId) -> EffectiveBudget {
        let ov = self.agent_overrides.get(&agent);
        EffectiveBudget {
            startup_max_lines: ov
                .and_then(|o| o.startup_max_lines)
                .unwrap_or(self.startup_max_lines),
            startup_max_bytes: ov
                .and_then(|o| o.startup_max_bytes)
                .unwrap_or(self.startup_max_bytes),
            default_recall_layer: ov
                .and_then(|o| o.default_recall_layer)
                .unwrap_or(self.default_recall_layer),
            max_recall_tokens: ov
                .and_then(|o| o.max_recall_tokens)
                .unwrap_or(self.max_recall_tokens),
            auto_expand_l1_threshold: self.auto_expand_l1_threshold,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LifecyclePolicy {
    pub category_mutability: HashMap<MemoryCategory, MutabilityMode>,
    pub archival: ArchivalPolicy,
    pub dreaming: DreamingPolicy,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        let mut category_mutability = HashMap::new();
        category_mutability.insert(MemoryCategory::Profile, MutabilityMode::Mutable);
        category_mutability.insert(MemoryCategory::Preferences, MutabilityMode::Mutable);
        category_mutability.insert(MemoryCategory::Entities, MutabilityMode::Mutable);
        category_mutability.insert(MemoryCategory::Events, MutabilityMode::Immutable);
        category_mutability.insert(MemoryCategory::Cases, MutabilityMode::Immutable);
        category_mutability.insert(MemoryCategory::Patterns, MutabilityMode::Immutable);

        Self {
            category_mutability,
            archival: ArchivalPolicy::default(),
            dreaming: DreamingPolicy::default(),
        }
    }
}

impl LifecyclePolicy {
    pub fn is_mutable(&self, category: MemoryCategory) -> bool {
        self.category_mutability
            .get(&category)
            .copied()
            .unwrap_or(MutabilityMode::Immutable)
            == MutabilityMode::Mutable
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MutabilityMode {
    Mutable,
    Immutable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchivalPolicy {
    pub enabled: bool,
    pub stale_days: u32,
    pub min_hit_count: u32,
    pub archive_uri: String,
}

impl Default for ArchivalPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            stale_days: 45,
            min_hit_count: 1,
            archive_uri: "viking://~/archive/".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReflectorMode {
    Deterministic,
    LocalCli,
    OpenaiCompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DreamingPolicy {
    pub enabled: bool,
    pub trigger: DreamingTrigger,
    pub cron: String,
    pub session_threshold: u32,
    pub dedupe_and_merge: bool,
    pub promote_hot_notes: bool,
    pub compile_wiki: bool,
    pub auto_snapshot_before_run: bool,
    pub require_human_review: bool,
    pub reflector_mode: ReflectorMode,
    pub reflector_cli_agent: String,
    pub reflector_endpoint: String,
    pub reflector_model: String,
    pub reflector_api_key: String,
}

impl Default for DreamingPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger: DreamingTrigger::Schedule,
            cron: "0 3 * * *".to_string(),
            session_threshold: 10,
            dedupe_and_merge: true,
            promote_hot_notes: true,
            compile_wiki: true,
            auto_snapshot_before_run: true,
            require_human_review: true,
            reflector_mode: ReflectorMode::Deterministic,
            reflector_cli_agent: "auto".to_string(),
            reflector_endpoint: "http://127.0.0.1:11434/v1".to_string(),
            reflector_model: "qwen2.5:7b".to_string(),
            reflector_api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DreamingTrigger {
    Schedule,
    SessionThreshold,
    Manual,
}

/// Real-time route decision returned by the Scope Router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedRoute {
    pub target_resource_uri: String,
    pub target_memory_uri: String,
    pub agent_private_memory_uri: Option<String>,
    pub matched_source: RouteMatchSource,
    pub is_fallback_inbox: bool,
    pub project_slug: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteMatchSource {
    ProjectPolicyFile,
    ManualMapping,
    GitRemoteAuto,
    InboxFallback,
}

/// Dreaming background consolidation proposal persisted in local SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMemoryChange {
    pub id: String,
    pub run_id: String,
    pub snapshot_id: Option<String>,
    pub created_at: i64,
    pub action: ChangeAction,
    pub risk_level: RiskLevel,
    pub target_uri: String,
    pub destination_uri: Option<String>,
    pub category: MemoryCategory,
    pub base_hash: Option<String>,
    pub before_content: Option<String>,
    pub after_content: Option<String>,
    pub rationale: String,
    pub evidence_json: String,
    pub status: ReviewStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Merge,
    MoveFromInbox,
    PromoteToIndex,
    DemoteFromIndex,
    Archive,
    WikiCompile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pending,
    Accepted,
    Rejected,
    Applied,
    Conflict,
}

impl ReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewStatus::Pending => "pending",
            ReviewStatus::Accepted => "accepted",
            ReviewStatus::Rejected => "rejected",
            ReviewStatus::Applied => "applied",
            ReviewStatus::Conflict => "conflict",
        }
    }
}
