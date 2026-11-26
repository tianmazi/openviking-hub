import type {
  AgentId,
  AgentStatus,
  ApplyReport,
  DreamingRun,
  LayeredContent,
  PendingMemoryChange,
  PolicyConfig,
  ProfileStore,
  RecallEvent,
  ReviewStatus,
  ServerStatus,
  WslDistroInfo,
} from "../types/openviking";

const isTauriRuntime = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function callTauri<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

let previewProfiles: ProfileStore = {
  active: "default",
  profiles: {
    default: {
      name: "default",
      label: "Local / VPS OpenViking",
      url: "http://127.0.0.1:1933",
      api_key: "",
    },
    volcengine: {
      name: "volcengine",
      label: "Volcano Engine Managed",
      url: "https://openviking.volcengine.example",
      api_key: "",
    },
  },
};

let previewPolicy: PolicyConfig = {
  schema_version: 1,
  global: {
    capture_enabled: true,
    capture_commit_tokens: 0,
    active_profile: "default",
    default_visibility: "private",
  },
  routing: {
    auto_map_git_repo: true,
    respect_project_policy_file: true,
    isolate_agent_namespaces: true,
    inbox_uri: "viking://~/resources/inbox/",
    protected_rule_globs: [
      "CLAUDE.md",
      "**/CLAUDE.md",
      "AGENTS.md",
      ".cursorrules",
      ".agents/rules/**",
    ],
    workspace_mappings: [
      {
        local_path: "~/work/openviking-desktop",
        resource_uri: "viking://resources/projects/openviking-desktop/",
        visibility: "shared",
      },
    ],
  },
  budget: {
    startup_max_lines: 200,
    startup_max_bytes: 24576,
    default_recall_layer: "l0",
    auto_expand_l1_threshold: 0.8,
    max_recall_tokens: 2000,
    category_quotas: {
      profile: 200,
      preferences: 500,
      entities: 500,
      events: 300,
      cases: 300,
      patterns: 200,
    },
    agent_overrides: {
      "claude-code": {
        startup_max_bytes: 9500,
        default_recall_layer: "l0",
        max_recall_tokens: 2500,
      },
      codex: { default_recall_layer: "l0", max_recall_tokens: 2000 },
      pi: { default_recall_layer: "l1", max_recall_tokens: 3000 },
      antigravity: { default_recall_layer: "l1", max_recall_tokens: 4000 },
    },
  },
  lifecycle: {
    category_mutability: {
      profile: "mutable",
      preferences: "mutable",
      entities: "mutable",
      events: "immutable",
      cases: "immutable",
      patterns: "immutable",
    },
    archival: {
      enabled: true,
      stale_days: 45,
      min_hit_count: 1,
      archive_uri: "viking://~/archive/",
    },
    dreaming: {
      enabled: true,
      trigger: "schedule",
      cron: "0 3 * * *",
      session_threshold: 10,
      dedupe_and_merge: true,
      promote_hot_notes: true,
      compile_wiki: true,
      auto_snapshot_before_run: true,
      require_human_review: true,
      reflector_mode: "deterministic",
      reflector_cli_agent: "auto",
      reflector_endpoint: "http://127.0.0.1:11434/v1",
      reflector_model: "qwen2.5:7b",
      reflector_api_key: "",
    },
  },
};

let previewChanges: PendingMemoryChange[] = [
  {
    id: "chg-demo-1",
    run_id: "dream-demo-01",
    created_at: Math.floor(Date.now() / 1000) - 3600,
    action: "merge",
    risk_level: "medium",
    target_uri: "viking://user/memories/preferences/git-commit.md",
    category: "preferences",
    base_hash: "e3b0c44298fc1c149afbf4c8996fb924",
    before_content: "# Git Commit 规范\n\n使用中文自由格式编写 commit message。",
    after_content:
      "# Git Commit 规范\n\n统一使用英文 Conventional Commits（feat/fix/chore/docs），保持标题 72 字符以内。",
    rationale: "检测到 Codex 与 Claude Code 近期会话要求使用 Conventional Commits，覆盖 10 月旧偏好。",
    evidence_json: JSON.stringify({
      hit_count: 14,
      agents: ["claude-code", "codex"],
      similarity: 0.91,
    }),
    status: "pending",
  },
  {
    id: "chg-demo-2",
    run_id: "dream-demo-01",
    created_at: Math.floor(Date.now() / 1000) - 3500,
    action: "move_from_inbox",
    risk_level: "medium",
    target_uri: "viking://~/resources/inbox/auth-v2-notes.md",
    destination_uri: "viking://resources/projects/openviking-desktop/auth-v2-notes.md",
    category: "entities",
    before_content: "# Auth V2\n使用 Authorization: Bearer + X-API-Key 双头兼容。",
    after_content: "# Auth V2\n使用 Authorization: Bearer + X-API-Key 双头兼容。",
    rationale: "Inbox 待分拣笔记与 openviking-desktop 仓库高度相关，建议移入项目目录。",
    evidence_json: JSON.stringify({ source: "inbox_sorter", agents: ["antigravity"] }),
    status: "pending",
  },
  {
    id: "chg-demo-3",
    run_id: "dream-demo-01",
    created_at: Math.floor(Date.now() / 1000) - 3400,
    action: "archive",
    risk_level: "low",
    target_uri: "viking://user/memories/entities/legacy-redis-node.md",
    destination_uri: "viking://~/archive/legacy-redis-node.md",
    category: "entities",
    before_content: "旧测试环境 Redis 节点信息（已下线）。",
    after_content: null,
    rationale: "已 52 天未被任何 Agent 命中（0 次召回），建议归档以减少上下文噪音。",
    evidence_json: JSON.stringify({ hit_count: 0, stale_days: 52, agents: [] }),
    status: "pending",
  },
  {
    id: "chg-demo-4",
    run_id: "dream-demo-01",
    created_at: Math.floor(Date.now() / 1000) - 3300,
    action: "wiki_compile",
    risk_level: "low",
    target_uri: "viking://resources/projects/",
    category: "patterns",
    after_content: "触发 OpenViking POST /api/v1/compile 将 3 个活跃项目编译为互联 Wiki。",
    rationale: "Wiki + 搜索组合可显著降低每次复杂任务的召回 Token 消耗。",
    evidence_json: JSON.stringify({ project_count: 3 }),
    status: "pending",
  },
];

export const api = {
  async getServerStatus(): Promise<ServerStatus> {
    if (isTauriRuntime()) return callTauri("get_server_status");
    const active = previewProfiles.profiles[previewProfiles.active];
    return {
      reachable: true,
      latency_ms: 18,
      url: active?.url ?? "http://127.0.0.1:1933",
      error: null,
    };
  },

  async testConnection(
    url: string,
    apiKey: string,
    account?: string,
    user?: string,
  ): Promise<ServerStatus> {
    if (isTauriRuntime()) {
      return callTauri("test_connection", {
        url,
        apiKey,
        account: account ?? null,
        user: user ?? null,
      });
    }
    return { reachable: true, latency_ms: 21, url, error: null };
  },

  async getProfiles(): Promise<ProfileStore> {
    if (isTauriRuntime()) return callTauri("get_profiles");
    return previewProfiles;
  },

  async saveProfiles(store: ProfileStore): Promise<ProfileStore> {
    if (isTauriRuntime()) return callTauri("save_profiles", { store });
    previewProfiles = store;
    return store;
  },

  async detectAgents(): Promise<AgentStatus[]> {
    if (isTauriRuntime()) return callTauri("detect_agents");
    return [
      {
        id: "claude-code",
        agent_id: "claude-code",
        name: "Claude Code",
        integration_mode: "MCP + Hooks",
        installed: true,
        connected: true,
        detected_surfaces: ["CLI"],
        binary_path: "/usr/local/bin/claude",
        config_path: "~/.claude.json",
        version: "claude 1.0.32",
        legacy_memory_files: ["~/.claude/CLAUDE.md"],
      },
      {
        id: "codex",
        agent_id: "codex",
        name: "Codex (Desktop App + CLI)",
        integration_mode: "Unified Global TOML (~/.codex/config.toml)",
        installed: true,
        connected: false,
        detected_surfaces: ["Desktop App", "CLI"],
        binary_path: "/Applications/Codex.app",
        config_path: "~/.codex/config.toml",
        version: "codex 0.14.0",
        warning:
          "Codex Desktop 与 CLI 共享全局 ~/.codex/config.toml（桌面端不读取项目级 .codex/config.toml）。注入后请重启 Codex Desktop 并新建线程生效。",
        legacy_memory_files: [],
      },
      {
        id: "pi",
        agent_id: "pi",
        name: "pi Coding Agent",
        integration_mode: "Native Extension Bridge",
        installed: true,
        connected: true,
        detected_surfaces: ["CLI"],
        binary_path: "/usr/local/bin/pi",
        config_path: "~/.pi/extensions/openviking-bridge",
        version: "pi 0.9.1",
        legacy_memory_files: [],
      },
      {
        id: "antigravity",
        agent_id: "antigravity",
        name: "Antigravity (反重力 Desktop / IDE / CLI)",
        integration_mode: "Global MCP + Hooks (mcp_config.json + hooks.json)",
        installed: true,
        connected: false,
        detected_surfaces: ["Desktop App 2.0", "CLI"],
        binary_path: "/Applications/Antigravity.app",
        config_path: "~/.gemini/config/mcp_config.json + ~/.gemini/config/hooks.json",
        warning:
          "Antigravity 2.0 桌面端与 CLI 共享全局配置。注入后请在桌面端 Customizations → Installed → MCP Servers 点击 Refresh 或重启应用生效。",
        legacy_memory_files: [],
      },
    ];
  },

  async detectWsl(): Promise<WslDistroInfo[]> {
    if (isTauriRuntime()) return callTauri("detect_wsl");
    return [];
  },

  async connectWslAgent(
    distroName: string,
    homeUnc: string,
    linuxBridgePath?: string,
  ): Promise<string[]> {
    if (isTauriRuntime()) {
      return callTauri("connect_wsl_agent", {
        distroName,
        homeUnc,
        linuxBridgePath: linuxBridgePath ?? null,
      });
    }
    return [`${homeUnc}/.claude.json`, `${homeUnc}/.codex/config.toml`];
  },

  async connectAgent(
    agentId: AgentId,
    writeProjectRules = false,
    projectDir?: string,
  ): Promise<AgentStatus> {
    if (isTauriRuntime()) {
      return callTauri("connect_agent", {
        agentId,
        writeProjectRules,
        projectDir: projectDir ?? null,
      });
    }
    const all = await this.detectAgents();
    const target = all.find((a) => a.agent_id === agentId) ?? all[0];
    return { ...target, connected: true };
  },

  async disconnectAgent(agentId: AgentId): Promise<AgentStatus> {
    if (isTauriRuntime()) return callTauri("disconnect_agent", { agentId });
    const all = await this.detectAgents();
    const target = all.find((a) => a.agent_id === agentId) ?? all[0];
    return { ...target, connected: false };
  },

  async importLegacyMemory(localPath: string, targetUri: string): Promise<unknown> {
    if (isTauriRuntime()) {
      return callTauri("import_legacy_memory", { localPath, targetUri });
    }
    return { status: "ok", localPath, targetUri };
  },

  async getPolicy(): Promise<PolicyConfig> {
    if (isTauriRuntime()) return callTauri("get_policy");
    return previewPolicy;
  },

  async savePolicy(policy: PolicyConfig): Promise<PolicyConfig> {
    if (isTauriRuntime()) return callTauri("save_policy", { policy });
    previewPolicy = policy;
    return policy;
  },

  async togglePrivacyMode(captureEnabled: boolean): Promise<PolicyConfig> {
    if (isTauriRuntime()) {
      return callTauri("toggle_privacy_mode", { captureEnabled });
    }
    previewPolicy = {
      ...previewPolicy,
      global: { ...previewPolicy.global, capture_enabled: captureEnabled },
    };
    return previewPolicy;
  },

  async fsLs(
    uri: string,
  ): Promise<Array<{ name: string; uri: string; isDir?: boolean; size?: number }>> {
    if (isTauriRuntime()) {
      const res = await callTauri<unknown>("fs_ls", { uri });
      const items = Array.isArray(res) ? res : [];
      return items.map((item: { name?: string; uri?: string; isDir?: boolean; size?: number }) => {
        const itemUri = item.uri ?? "";
        const derivedName =
          item.name ?? (itemUri.replace(/\/+$/, "").split("/").pop() || itemUri);
        return {
          name: item.isDir && !derivedName.endsWith("/") ? `${derivedName}/` : derivedName,
          uri: itemUri,
          isDir: item.isDir,
          size: item.size,
        };
      });
    }
    if (uri === "viking://~/memories/" || uri === "viking://user/memories/") {
      return [
        {
          name: "preferences/git-commit.md",
          uri: "viking://~/memories/preferences/git-commit.md",
          isDir: false,
          size: 412,
        },
        {
          name: "profile/coding-style.md",
          uri: "viking://~/memories/profile/coding-style.md",
          isDir: false,
          size: 680,
        },
        {
          name: "entities/openviking-cluster.md",
          uri: "viking://~/memories/entities/openviking-cluster.md",
          isDir: false,
          size: 520,
        },
      ];
    }
    if (uri === "viking://agent/") {
      return [
        { name: "claude-code/memories/", uri: "viking://agent/claude-code/memories/", isDir: true },
        { name: "codex/memories/", uri: "viking://agent/codex/memories/", isDir: true },
        { name: "antigravity/memories/", uri: "viking://agent/antigravity/memories/", isDir: true },
      ];
    }
    if (uri === "viking://~/resources/inbox/") {
      return [
        {
          name: "auth-v2-notes.md",
          uri: "viking://~/resources/inbox/auth-v2-notes.md",
          isDir: false,
          size: 245,
        },
      ];
    }
    return [
      {
        name: "openviking-desktop/",
        uri: "viking://resources/projects/openviking-desktop/",
        isDir: true,
      },
      {
        name: "ARCHITECTURE.md",
        uri: "viking://resources/projects/openviking-desktop/ARCHITECTURE.md",
        isDir: false,
        size: 2410,
      },
    ];
  },

  async fsDelete(uri: string, recursive = false): Promise<unknown> {
    if (isTauriRuntime()) return callTauri("fs_delete", { uri, recursive });
    return { status: "ok", uri };
  },

  async searchFind(
    query: string,
    targetUri?: string,
  ): Promise<Array<{ name: string; uri: string; isDir?: boolean }>> {
    if (isTauriRuntime()) {
      const res = await callTauri<unknown>("search_find", {
        query,
        targetUri: targetUri ?? null,
        limit: 20,
      });
      const rec = (res && typeof res === "object" ? res : {}) as Record<string, unknown>;
      const arr = Array.isArray(res)
        ? res
        : [
            ...(Array.isArray(rec.items) ? rec.items : []),
            ...(Array.isArray(rec.results) ? rec.results : []),
            ...(Array.isArray(rec.memories) ? rec.memories : []),
            ...(Array.isArray(rec.resources) ? rec.resources : []),
            ...(Array.isArray(rec.skills) ? rec.skills : []),
          ];
      return arr.map((item: { uri?: string; name?: string }) => ({
        name: item.name ?? item.uri?.split("/").pop() ?? "result",
        uri: item.uri ?? "",
        isDir: false,
      }));
    }
    return [
      {
        name: `命中结果: ${query} (git-commit.md)`,
        uri: "viking://user/memories/preferences/git-commit.md",
        isDir: false,
      },
    ];
  },

  async readUriLayers(uri: string): Promise<LayeredContent> {
    if (isTauriRuntime()) return callTauri("read_uri_layers", { uri });
    return {
      uri,
      l0: `L0 摘要 (~80 tokens): ${uri} 的核心约束与索引摘要，供 Hook 自动召回快速过滤。`,
      l1: `## L1 概览 (~1.2k tokens)\n\n- 目标 URI: \`${uri}\`\n- 适用范围: 跨 Claude Code / Codex / pi / Antigravity 共享\n- 关键条目: 偏好规范、命名空间边界、回滚快照策略`,
      l2: `# ${uri.split("/").pop() || "Memory"}\n\n这里展示完整的 L2 原文 Markdown，支持在桌面端直接修改并通过 \`POST /api/v1/content/write\` 写回远端并刷新语义索引。`,
    };
  },

  async writeUriContent(uri: string, content: string): Promise<unknown> {
    if (isTauriRuntime()) return callTauri("write_uri_content", { uri, content });
    return { status: "ok", uri };
  },

  async listRecallEvents(agentFilter?: string): Promise<RecallEvent[]> {
    if (isTauriRuntime()) {
      return callTauri("list_recall_events", { agentFilter: agentFilter ?? null, limit: 100 });
    }
    const now = Math.floor(Date.now() / 1000);
    const items: RecallEvent[] = [
      {
        id: "ev-1",
        agent_id: "claude-code",
        event_type: "recall",
        query: "OpenViking snapshot commit API endpoint",
        target_uri: "viking://resources/projects/openviking-desktop/",
        layer: "l0",
        tokens_used: 340,
        latency_ms: 42,
        blocked_by_privacy: false,
        created_at: now - 120,
      },
      {
        id: "ev-2",
        agent_id: "codex",
        event_type: "capture",
        query: null,
        target_uri: "viking://user/memories/preferences/git-commit.md",
        layer: "l2",
        tokens_used: 185,
        latency_ms: 68,
        blocked_by_privacy: false,
        created_at: now - 480,
      },
      {
        id: "ev-3",
        agent_id: "antigravity",
        event_type: "recall",
        query: "Tauri v2 tray icon macOS template",
        target_uri: "viking://resources/projects/openviking-desktop/",
        layer: "l1",
        tokens_used: 1120,
        latency_ms: 95,
        blocked_by_privacy: false,
        created_at: now - 900,
      },
    ];
    return agentFilter ? items.filter((i) => i.agent_id === agentFilter) : items;
  },

  async triggerDreaming(): Promise<[DreamingRun, PendingMemoryChange[]]> {
    if (isTauriRuntime()) return callTauri("trigger_dreaming");
    return [
      {
        id: "dream-demo-01",
        trigger_source: "manual",
        status: "ready_for_review",
        snapshot_id: null,
        started_at: Math.floor(Date.now() / 1000) - 5,
        finished_at: Math.floor(Date.now() / 1000),
        summary_json: '{"merge":1,"inbox":1,"archive":1,"wiki_compile":1}',
      },
      previewChanges,
    ];
  },

  async listDreamingRuns(): Promise<DreamingRun[]> {
    if (isTauriRuntime()) return callTauri("list_dreaming_runs", { limit: 20 });
    return [
      {
        id: "dream-demo-01",
        trigger_source: "schedule",
        status: "ready_for_review",
        snapshot_id: "snap-20261126-0300",
        started_at: Math.floor(Date.now() / 1000) - 3600,
        finished_at: Math.floor(Date.now() / 1000) - 3580,
        summary_json: '{"merge":1,"inbox":1,"archive":1,"wiki_compile":1}',
      },
    ];
  },

  async listPendingChanges(): Promise<PendingMemoryChange[]> {
    if (isTauriRuntime()) return callTauri("list_pending_changes", { statusFilter: null });
    return [...previewChanges];
  },

  async updateChangeDecision(
    id: string,
    status: ReviewStatus,
    editedAfterContent?: string,
  ): Promise<void> {
    if (isTauriRuntime()) {
      return callTauri("update_change_decision", {
        id,
        status,
        editedAfterContent: editedAfterContent ?? null,
      });
    }
    previewChanges = previewChanges.map((c) =>
      c.id === id
        ? { ...c, status, after_content: editedAfterContent ?? c.after_content }
        : c,
    );
  },

  async applyAcceptedChanges(): Promise<ApplyReport> {
    if (isTauriRuntime()) return callTauri("apply_accepted_changes");
    const accepted = previewChanges.filter((c) => c.status === "accepted");
    previewChanges = previewChanges.map((c) =>
      c.status === "accepted" ? { ...c, status: "applied" } : c,
    );
    return {
      snapshot_id: "snap-" + Date.now(),
      applied_ids: accepted.map((c) => c.id),
      conflict_ids: [],
      failed_items: [],
    };
  },

  async rollbackDreamingRun(runId: string): Promise<unknown> {
    if (isTauriRuntime()) return callTauri("rollback_dreaming_run", { runId });
    return { status: "rolled_back", runId };
  },

  async pinMemoryUri(uri: string): Promise<void> {
    if (isTauriRuntime()) return callTauri("pin_memory_uri", { uri });
  },
};
