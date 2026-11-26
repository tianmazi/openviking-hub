import { useState } from "react";
import { Plus, Save, Trash2 } from "lucide-react";
import type {
  AgentId,
  ContextLayer,
  DreamingTrigger,
  MemoryCategory,
  MutabilityMode,
  PolicyConfig,
  VisibilityScope,
} from "../types/openviking";

interface PolicyViewProps {
  policy: PolicyConfig;
  onSave: (next: PolicyConfig) => Promise<void>;
}

const AGENT_LABELS: Record<AgentId, string> = {
  "claude-code": "Claude Code",
  codex: "Codex CLI",
  pi: "pi Coding Agent",
  antigravity: "Antigravity (反重力)",
};

const CATEGORIES: Array<{ id: MemoryCategory; label: string }> = [
  { id: "profile", label: "Profile（用户画像）" },
  { id: "preferences", label: "Preferences（偏好习惯）" },
  { id: "entities", label: "Entities（项目/实体）" },
  { id: "events", label: "Events（历史事件）" },
  { id: "cases", label: "Cases（踩坑案例）" },
  { id: "patterns", label: "Patterns（复用模式）" },
];

export function PolicyView({ policy, onSave }: PolicyViewProps) {
  const [draft, setDraft] = useState<PolicyConfig>(policy);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [newMapPath, setNewMapPath] = useState("");
  const [newMapUri, setNewMapUri] = useState("viking://resources/projects/");
  const [newMapVis, setNewMapVis] = useState<VisibilityScope>("shared");
  const [newGlob, setNewGlob] = useState("");

  const handleSave = async () => {
    setSaving(true);
    setMsg(null);
    try {
      await onSave(draft);
      setMsg("策略已原子写入 ~/.openviking/policy.toml，ov-bridge 将在下次调用时自动热加载。");
    } catch (e) {
      setMsg(`保存失败：${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const addMapping = () => {
    if (!newMapPath.trim() || !newMapUri.startsWith("viking://")) return;
    setDraft({
      ...draft,
      routing: {
        ...draft.routing,
        workspace_mappings: [
          ...draft.routing.workspace_mappings,
          {
            local_path: newMapPath.trim(),
            resource_uri: newMapUri.trim(),
            visibility: newMapVis,
          },
        ],
      },
    });
    setNewMapPath("");
  };

  const addGlob = () => {
    if (!newGlob.trim()) return;
    setDraft({
      ...draft,
      routing: {
        ...draft.routing,
        protected_rule_globs: [...draft.routing.protected_rule_globs, newGlob.trim()],
      },
    });
    setNewGlob("");
  };

  return (
    <div className="h-full overflow-y-auto p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-zinc-100">内置记忆治理策略中心</h1>
          <p className="text-xs text-zinc-400 mt-1">
            配置作用域路由、分层 Token 预算、六类记忆配额/可变性以及 Dreaming 后台异步整理规则（落盘于{" "}
            <code className="font-mono text-zinc-300">~/.openviking/policy.toml</code>）。
          </p>
        </div>
        <button
          onClick={handleSave}
          disabled={saving}
          className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-xs font-medium text-white"
        >
          <Save className="w-3.5 h-3.5" />
          {saving ? "保存中..." : "保存策略"}
        </button>
      </div>

      {msg && (
        <div className="px-3.5 py-2.5 rounded-lg bg-indigo-500/10 border border-indigo-500/25 text-xs text-indigo-200">
          {msg}
        </div>
      )}

      {/* 1. Scope Router */}
      <section className="rounded-xl bg-surface-1 border border-border-subtle p-5 space-y-4">
        <h2 className="text-sm font-semibold text-zinc-100">1. 作用域路由与规则保护（Scope Router）</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
          <label className="flex items-center justify-between p-3 rounded-lg bg-surface-2 border border-border-subtle">
            <span>自动识别 Git Remote 映射到项目目录</span>
            <input
              type="checkbox"
              checked={draft.routing.auto_map_git_repo}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  routing: { ...draft.routing, auto_map_git_repo: e.target.checked },
                })
              }
            />
          </label>
          <label className="flex items-center justify-between p-3 rounded-lg bg-surface-2 border border-border-subtle">
            <span>优先读取项目根目录 RESOURCE_POLICY.md</span>
            <input
              type="checkbox"
              checked={draft.routing.respect_project_policy_file}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  routing: { ...draft.routing, respect_project_policy_file: e.target.checked },
                })
              }
            />
          </label>
        </div>

        <div className="space-y-1.5">
          <label className="text-xs text-zinc-400">Inbox 兜底目录 URI</label>
          <input
            value={draft.routing.inbox_uri}
            onChange={(e) =>
              setDraft({ ...draft, routing: { ...draft.routing, inbox_uri: e.target.value } })
            }
            className="w-full bg-surface-2 border border-border-subtle rounded-lg px-3 py-2 font-mono text-xs text-zinc-200"
          />
        </div>

        <div className="space-y-2">
          <div className="text-xs font-medium text-zinc-300">手动目录映射（优先于 Git 自动识别）</div>
          <div className="space-y-1.5">
            {draft.routing.workspace_mappings.map((m, idx) => (
              <div
                key={`${m.local_path}-${idx}`}
                className="flex items-center justify-between gap-2 px-3 py-1.5 rounded-lg bg-surface-2 border border-border-subtle font-mono text-[11px]"
              >
                <span className="text-zinc-200 truncate">{m.local_path}</span>
                <span className="text-zinc-500">→</span>
                <span className="text-indigo-300 truncate flex-1">{m.resource_uri}</span>
                <span className="px-1.5 py-0.5 rounded bg-surface-1 text-[10px] text-zinc-400">
                  {m.visibility}
                </span>
                <button
                  onClick={() =>
                    setDraft({
                      ...draft,
                      routing: {
                        ...draft.routing,
                        workspace_mappings: draft.routing.workspace_mappings.filter(
                          (_, i) => i !== idx,
                        ),
                      },
                    })
                  }
                  className="text-zinc-500 hover:text-rose-400"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
          <div className="flex flex-wrap gap-2">
            <input
              value={newMapPath}
              onChange={(e) => setNewMapPath(e.target.value)}
              placeholder="~/work/my-repo"
              className="flex-1 min-w-[160px] bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-xs text-zinc-200"
            />
            <input
              value={newMapUri}
              onChange={(e) => setNewMapUri(e.target.value)}
              placeholder="viking://resources/projects/my-repo/"
              className="flex-1 min-w-[220px] bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-xs text-zinc-200"
            />
            <select
              value={newMapVis}
              onChange={(e) => setNewMapVis(e.target.value as VisibilityScope)}
              className="bg-surface-2 border border-border-subtle rounded-lg px-2 py-1.5 text-xs text-zinc-200"
            >
              <option value="shared">shared</option>
              <option value="private">private</option>
            </select>
            <button
              onClick={addMapping}
              className="inline-flex items-center gap-1 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-200"
            >
              <Plus className="w-3.5 h-3.5" />
              添加
            </button>
          </div>
        </div>

        <div className="space-y-2">
          <div className="text-xs font-medium text-zinc-300">人工强约束规则只读保护（protected_rule_globs）</div>
          <div className="flex flex-wrap gap-1.5">
            {draft.routing.protected_rule_globs.map((g, idx) => (
              <span
                key={`${g}-${idx}`}
                className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-surface-2 border border-border-subtle font-mono text-[11px] text-zinc-300"
              >
                {g}
                <button
                  onClick={() =>
                    setDraft({
                      ...draft,
                      routing: {
                        ...draft.routing,
                        protected_rule_globs: draft.routing.protected_rule_globs.filter(
                          (_, i) => i !== idx,
                        ),
                      },
                    })
                  }
                  className="text-zinc-500 hover:text-rose-400"
                >
                  ×
                </button>
              </span>
            ))}
          </div>
          <div className="flex gap-2">
            <input
              value={newGlob}
              onChange={(e) => setNewGlob(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addGlob()}
              placeholder="新增保护 Glob，如 **/SECURITY.md"
              className="flex-1 bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-xs text-zinc-200"
            />
            <button
              onClick={addGlob}
              className="inline-flex items-center gap-1 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-200"
            >
              <Plus className="w-3.5 h-3.5" />
              添加
            </button>
          </div>
        </div>
      </section>

      {/* 2. Tiered Token Budget + Six-Category Quotas & Mutability */}
      <section className="rounded-xl bg-surface-1 border border-border-subtle p-5 space-y-5">
        <h2 className="text-sm font-semibold text-zinc-100">2. 分层注入与按 Agent Token 预算</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {(Object.keys(AGENT_LABELS) as AgentId[]).map((agentId) => {
            const ov = draft.budget.agent_overrides[agentId] ?? {};
            const tokens = ov.max_recall_tokens ?? draft.budget.max_recall_tokens;
            const layer = ov.default_recall_layer ?? draft.budget.default_recall_layer;
            return (
              <div
                key={agentId}
                className="p-3.5 rounded-lg bg-surface-2 border border-border-subtle space-y-3"
              >
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-zinc-200">
                    {AGENT_LABELS[agentId]}
                  </span>
                  <span className="font-mono text-xs text-indigo-300">{tokens} tokens</span>
                </div>
                <input
                  type="range"
                  min={500}
                  max={6000}
                  step={100}
                  value={tokens}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      budget: {
                        ...draft.budget,
                        agent_overrides: {
                          ...draft.budget.agent_overrides,
                          [agentId]: { ...ov, max_recall_tokens: Number(e.target.value) },
                        },
                      },
                    })
                  }
                  className="w-full accent-indigo-500"
                />
                <div className="flex items-center justify-between text-[11px] text-zinc-400">
                  <span>默认召回层级</span>
                  <select
                    value={layer}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        budget: {
                          ...draft.budget,
                          agent_overrides: {
                            ...draft.budget.agent_overrides,
                            [agentId]: {
                              ...ov,
                              default_recall_layer: e.target.value as ContextLayer,
                            },
                          },
                        },
                      })
                    }
                    className="bg-surface-1 border border-border-subtle rounded px-2 py-0.5 font-mono text-zinc-200"
                  >
                    <option value="l0">L0 (~100t 摘要)</option>
                    <option value="l1">L1 (~2k 概览)</option>
                    <option value="l2">L2 (全文)</option>
                  </select>
                </div>
              </div>
            );
          })}
        </div>

        <div className="space-y-2">
          <div className="text-xs font-medium text-zinc-300">六类记忆独立配额与更新策略</div>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5 text-xs">
            {CATEGORIES.map((cat) => (
              <div
                key={cat.id}
                className="p-3 rounded-lg bg-surface-2 border border-border-subtle space-y-2"
              >
                <div className="text-[11px] font-medium text-zinc-200">{cat.label}</div>
                <div className="flex items-center justify-between gap-2">
                  <input
                    type="number"
                    min={50}
                    step={50}
                    value={draft.budget.category_quotas[cat.id] ?? 200}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        budget: {
                          ...draft.budget,
                          category_quotas: {
                            ...draft.budget.category_quotas,
                            [cat.id]: Number(e.target.value),
                          },
                        },
                      })
                    }
                    className="w-24 bg-surface-1 border border-border-subtle rounded px-2 py-1 font-mono text-[11px] text-zinc-200"
                  />
                  <select
                    value={draft.lifecycle.category_mutability[cat.id] ?? "immutable"}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        lifecycle: {
                          ...draft.lifecycle,
                          category_mutability: {
                            ...draft.lifecycle.category_mutability,
                            [cat.id]: e.target.value as MutabilityMode,
                          },
                        },
                      })
                    }
                    className="bg-surface-1 border border-border-subtle rounded px-2 py-1 font-mono text-[11px] text-zinc-200"
                  >
                    <option value="mutable">mutable（可合并）</option>
                    <option value="immutable">immutable（仅归档）</option>
                  </select>
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* 3. Dreaming & Archival */}
      <section className="rounded-xl bg-surface-1 border border-border-subtle p-5 space-y-4">
        <h2 className="text-sm font-semibold text-zinc-100">3. Dreaming 后台整理与冷记忆归档</h2>
        <div className="grid grid-cols-1 md:grid-cols-4 gap-4 text-xs">
          <div className="space-y-1.5">
            <label className="text-zinc-400">触发模式</label>
            <select
              value={draft.lifecycle.dreaming.trigger}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  lifecycle: {
                    ...draft.lifecycle,
                    dreaming: {
                      ...draft.lifecycle.dreaming,
                      trigger: e.target.value as DreamingTrigger,
                    },
                  },
                })
              }
              className="w-full bg-surface-2 border border-border-subtle rounded-lg px-3 py-2 text-zinc-200"
            >
              <option value="schedule">Cron 定时</option>
              <option value="session_threshold">累计会话数阈值</option>
              <option value="manual">仅手动触发</option>
            </select>
          </div>
          <div className="space-y-1.5">
            <label className="text-zinc-400">定时 Cron / 会话阈值</label>
            {draft.lifecycle.dreaming.trigger === "session_threshold" ? (
              <input
                type="number"
                min={1}
                value={draft.lifecycle.dreaming.session_threshold}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    lifecycle: {
                      ...draft.lifecycle,
                      dreaming: {
                        ...draft.lifecycle.dreaming,
                        session_threshold: Number(e.target.value),
                      },
                    },
                  })
                }
                className="w-full bg-surface-2 border border-border-subtle rounded-lg px-3 py-2 font-mono text-zinc-200"
              />
            ) : (
              <input
                value={draft.lifecycle.dreaming.cron}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    lifecycle: {
                      ...draft.lifecycle,
                      dreaming: { ...draft.lifecycle.dreaming, cron: e.target.value },
                    },
                  })
                }
                className="w-full bg-surface-2 border border-border-subtle rounded-lg px-3 py-2 font-mono text-zinc-200"
              />
            )}
          </div>
          <div className="space-y-1.5">
            <label className="text-zinc-400">冷记忆归档天数</label>
            <input
              type="number"
              min={7}
              value={draft.lifecycle.archival.stale_days}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  lifecycle: {
                    ...draft.lifecycle,
                    archival: {
                      ...draft.lifecycle.archival,
                      stale_days: Number(e.target.value),
                    },
                  },
                })
              }
              className="w-full bg-surface-2 border border-border-subtle rounded-lg px-3 py-2 font-mono text-zinc-200"
            />
          </div>
          <div className="space-y-1.5">
            <label className="text-zinc-400">归档目标 URI</label>
            <input
              value={draft.lifecycle.archival.archive_uri}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  lifecycle: {
                    ...draft.lifecycle,
                    archival: {
                      ...draft.lifecycle.archival,
                      archive_uri: e.target.value,
                    },
                  },
                })
              }
              className="w-full bg-surface-2 border border-border-subtle rounded-lg px-3 py-2 font-mono text-zinc-200"
            />
          </div>
        </div>
        <div className="flex flex-wrap gap-4 text-xs text-zinc-300">
          <label className="inline-flex items-center gap-2">
            <input
              type="checkbox"
              checked={draft.lifecycle.dreaming.auto_snapshot_before_run}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  lifecycle: {
                    ...draft.lifecycle,
                    dreaming: {
                      ...draft.lifecycle.dreaming,
                      auto_snapshot_before_run: e.target.checked,
                    },
                  },
                })
              }
            />
            Commit 前自动创建 OpenViking Snapshot（支持一键回滚）
          </label>
          <label className="inline-flex items-center gap-2">
            <input
              type="checkbox"
              checked={draft.lifecycle.dreaming.require_human_review}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  lifecycle: {
                    ...draft.lifecycle,
                    dreaming: {
                      ...draft.lifecycle.dreaming,
                      require_human_review: e.target.checked,
                    },
                  },
                })
              }
            />
            启用人工审核台（Stage-then-Commit）
          </label>
          <label className="inline-flex items-center gap-2">
            <input
              type="checkbox"
              checked={draft.lifecycle.dreaming.compile_wiki}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  lifecycle: {
                    ...draft.lifecycle,
                    dreaming: { ...draft.lifecycle.dreaming, compile_wiki: e.target.checked },
                  },
                })
              }
            />
            定期触发 Wiki 编译（POST /api/v1/compile）
          </label>
        </div>
      </section>
    </div>
  );
}
