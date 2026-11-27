export type AgentId = "claude-code" | "codex" | "pi" | "antigravity";
export type ContextLayer = "l0" | "l1" | "l2";
export type VisibilityScope = "shared" | "private";
export type ReflectorMode = "deterministic" | "local_cli" | "openai_compatible";
export type MemoryCategory =
  | "profile"
  | "preferences"
  | "entities"
  | "events"
  | "cases"
  | "patterns";
export type MutabilityMode = "mutable" | "immutable";
export type DreamingTrigger = "schedule" | "session_threshold" | "manual";
export type ChangeAction =
  | "merge"
  | "move_from_inbox"
  | "promote_to_index"
  | "demote_from_index"
  | "archive"
  | "wiki_compile";
export type RiskLevel = "low" | "medium" | "high";
export type ReviewStatus =
  | "pending"
  | "accepted"
  | "rejected"
  | "applied"
  | "conflict";

export interface ServerStatus {
  reachable: boolean;
  latency_ms: number;
  url: string;
  system_status?: unknown;
  observer?: unknown;
  error?: string | null;
}

export interface OvProfile {
  name: string;
  label: string;
  url: string;
  api_key: string;
  account?: string | null;
  user?: string | null;
}

export interface ProfileStore {
  active: string;
  profiles: Record<string, OvProfile>;
}

export interface CaptureHealth {
  pending: number;
  sent: number;
  uncertain_sessions: number;
  extraction_tasks: [string, string][];
  last_error: string | null;
}

export interface AgentStatus {
  id: string;
  agent_id: AgentId;
  name: string;
  integration_mode: string;
  installed: boolean;
  connected: boolean;
  detected_surfaces: string[];
  binary_path?: string | null;
  config_path?: string | null;
  version?: string | null;
  warning?: string | null;
  legacy_memory_files: string[];
}

export interface WslDistroInfo {
  name: string;
  home_unc?: string | null;
}

export interface WorkspaceMapping {
  local_path: string;
  resource_uri: string;
  visibility: VisibilityScope;
}

export interface AgentBudgetOverride {
  startup_max_lines?: number | null;
  startup_max_bytes?: number | null;
  default_recall_layer?: ContextLayer | null;
  max_recall_tokens?: number | null;
}

export interface PolicyConfig {
  schema_version: number;
  global: {
    capture_enabled: boolean;
    capture_commit_tokens: number;
    active_profile: string;
    default_visibility: VisibilityScope;
  };
  routing: {
    auto_map_git_repo: boolean;
    respect_project_policy_file: boolean;
    isolate_agent_namespaces: boolean;
    inbox_uri: string;
    protected_rule_globs: string[];
    workspace_mappings: WorkspaceMapping[];
  };
  budget: {
    startup_max_lines: number;
    startup_max_bytes: number;
    default_recall_layer: ContextLayer;
    auto_expand_l1_threshold: number;
    max_recall_tokens: number;
    min_recall_score: number;
    max_recall_items: number;
    category_quotas: Record<MemoryCategory, number>;
    agent_overrides: Partial<Record<AgentId, AgentBudgetOverride>>;
  };
  lifecycle: {
    category_mutability: Record<MemoryCategory, MutabilityMode>;
    archival: {
      enabled: boolean;
      stale_days: number;
      min_hit_count: number;
      archive_uri: string;
      respect_ttl_expiry?: boolean;
      episode_default_ttl_days?: number;
    };
    dreaming: {
      enabled: boolean;
      trigger: DreamingTrigger;
      cron: string;
      session_threshold: number;
      dedupe_and_merge: boolean;
      promote_hot_notes: boolean;
      demote_cold_index?: boolean;
      demote_stale_days?: number;
      max_index_entry_bytes?: number;
      incremental_mode?: boolean;
      verify_against_transcripts?: boolean;
      compile_wiki: boolean;
      auto_snapshot_before_run: boolean;
      require_human_review: boolean;
      auto_apply_low_risk: boolean;
      reflector_mode: ReflectorMode;
      reflector_cli_agent: string;
      reflector_endpoint: string;
      reflector_model: string;
      reflector_api_key: string;
    };
  };
}

export interface LayeredContent {
  uri: string;
  l0: string;
  l1: string;
  l2: string;
}

export interface RecallEvent {
  id: string;
  agent_id: string;
  event_type: string;
  query?: string | null;
  target_uri?: string | null;
  layer?: string | null;
  tokens_used: number;
  latency_ms: number;
  blocked_by_privacy: boolean;
  created_at: number;
}

export interface DreamingRun {
  id: string;
  trigger_source: string;
  status: string;
  snapshot_id?: string | null;
  started_at: number;
  finished_at?: number | null;
  summary_json: string;
}

export interface PendingMemoryChange {
  id: string;
  run_id: string;
  snapshot_id?: string | null;
  created_at: number;
  action: ChangeAction;
  risk_level: RiskLevel;
  target_uri: string;
  destination_uri?: string | null;
  category: MemoryCategory;
  base_hash?: string | null;
  before_content?: string | null;
  after_content?: string | null;
  rationale: string;
  evidence_json: string;
  status: ReviewStatus;
}

export interface ApplyReport {
  snapshot_id?: string | null;
  applied_ids: string[];
  conflict_ids: string[];
  failed_items: [string, string][];
}
