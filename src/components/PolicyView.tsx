import { useState } from "react";
import { Save } from "lucide-react";
import type { AgentId, ContextLayer, PolicyConfig } from "../types/openviking";

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

export function PolicyView({ policy, onSave }: PolicyViewProps) {
  const [draft, setDraft] = useState<PolicyConfig>(policy);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

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

  return (
    <div className="h-full overflow-y-auto p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-zinc-100">内置记忆治理策略中心</h1>
          <p className="text-xs text-zinc-400 mt-1">
            配置作用域路由、分层 Token 预算以及 Dreaming 后台异步整理规则（落盘于{" "}
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
      </section>

      {/* 2. Tiered Token Budget */}
      <section className="rounded-xl bg-surface-1 border border-border-subtle p-5 space-y-4">
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
                  <span className="text-xs font-semibold text-zinc-200">{AGENT_LABELS[agentId]}</span>
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
      </section>

      {/* 3. Dreaming & Archival */}
      <section className="rounded-xl bg-surface-1 border border-border-subtle p-5 space-y-4">
        <h2 className="text-sm font-semibold text-zinc-100">3. Dreaming 后台整理与冷记忆归档</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-xs">
          <div className="space-y-1.5">
            <label className="text-zinc-400">定时 Cron 表达式</label>
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
          </div>
          <div className="space-y-1.5">
            <label className="text-zinc-400">冷记忆归档天数（stale_days）</label>
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
