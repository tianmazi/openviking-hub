import { useEffect, useState } from "react";
import {
  Activity,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Cpu,
  Database,
  Layers,
  RefreshCw,
  Search,
  ShieldOff,
  XCircle,
} from "lucide-react";
import { api } from "../lib/tauri";
import type {
  AgentStatus,
  RecallEvent,
  RecallHitRecord,
  ServerStatus,
} from "../types/openviking";
import { MemoryFlowCanvas } from "./MemoryFlowCanvas";

const AGENT_FILTERS = [
  { id: "", label: "全部 Agent" },
  { id: "claude-code", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "pi" },
  { id: "antigravity", label: "Antigravity" },
];

const REASON_META: Record<string, { label: string; cls: string }> = {
  injected: {
    label: "已注入 (L0)",
    cls: "bg-emerald-500/15 text-emerald-300 border border-emerald-500/25",
  },
  injected_l1: {
    label: "已注入 (自动展开 L1)",
    cls: "bg-emerald-500/20 text-emerald-200 border border-emerald-500/35",
  },
  injected_l2: {
    label: "已注入 (L2 全文)",
    cls: "bg-emerald-500/20 text-emerald-200 border border-emerald-500/35",
  },
  below_min_score: {
    label: "低于相似度门控",
    cls: "bg-zinc-800 text-zinc-400 border border-border-subtle",
  },
  superseded: {
    label: "已被新版本替代",
    cls: "bg-violet-500/15 text-violet-300 border border-violet-500/25",
  },
  expired: {
    label: "TTL 已过期",
    cls: "bg-orange-500/15 text-orange-300 border border-orange-500/25",
  },
  max_items_reached: {
    label: "达到单次条数上限",
    cls: "bg-amber-500/15 text-amber-300 border border-amber-500/25",
  },
  category_quota: {
    label: "超出类别配额",
    cls: "bg-amber-500/15 text-amber-300 border border-amber-500/25",
  },
  token_budget: {
    label: "超出 Token 预算",
    cls: "bg-amber-500/15 text-amber-300 border border-amber-500/25",
  },
  skipped: {
    label: "已跳过",
    cls: "bg-zinc-800 text-zinc-400 border border-border-subtle",
  },
};

interface EventsViewProps {
  agents: AgentStatus[];
  captureEnabled: boolean;
}

export function EventsView({ agents, captureEnabled }: EventsViewProps) {
  const [agentFilter, setAgentFilter] = useState("");
  const [events, setEvents] = useState<RecallEvent[]>([]);
  const [status, setStatus] = useState<ServerStatus | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [hitsByEvent, setHitsByEvent] = useState<Record<string, RecallHitRecord[]>>({});
  const [loadingHitsId, setLoadingHitsId] = useState<string | null>(null);

  const load = () => {
    void api.listRecallEvents(agentFilter || undefined).then(setEvents);
    void api.getServerStatus().then(setStatus);
  };

  useEffect(() => {
    load();
    const timer = window.setInterval(load, 5000);
    return () => window.clearInterval(timer);
  }, [agentFilter]);

  const toggleInspect = async (eventId: string) => {
    if (expandedId === eventId) {
      setExpandedId(null);
      return;
    }
    setExpandedId(eventId);
    setLoadingHitsId(eventId);
    try {
      const hits = await api.listRecallHits(eventId);
      setHitsByEvent((prev) => ({ ...prev, [eventId]: hits }));
    } finally {
      setLoadingHitsId((curr) => (curr === eventId ? null : curr));
    }
  };

  const observer = (status?.observer ?? {}) as Record<string, unknown>;
  const components = (observer.components ?? {}) as Record<
    string,
    { is_healthy?: boolean; has_errors?: boolean }
  >;
  const queueLabel = components.queue
    ? components.queue.is_healthy
      ? "healthy"
      : "degraded"
    : String(observer.queue ?? "idle");
  const vikingdbLabel = components.vikingdb
    ? components.vikingdb.is_healthy
      ? "healthy"
      : "degraded"
    : String(observer.vikingdb ?? "ready");
  const totalTokens = events.reduce((acc, e) => acc + e.tokens_used, 0);

  return (
    <div className="h-full overflow-y-auto p-6 space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-zinc-100">实时召回事件流与 Context Inspector</h1>
          <p className="text-xs text-zinc-400 mt-0.5">
            追踪每个 Agent 通过 <code className="font-mono text-indigo-300">ov-bridge</code> 触发的 Recall / Capture；点击任意事件展开召回透视，查看候选 URI、相似度与注入/跳过原因。
          </p>
        </div>
        <button
          onClick={load}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-200"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          刷新
        </button>
      </div>

      {/* Live Animated Data-Flow Beam Canvas */}
      <MemoryFlowCanvas
        agents={agents}
        events={events}
        captureEnabled={captureEnabled}
        serverReachable={status?.reachable ?? false}
      />

      {/* Observer & Runtime Telemetry Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <div className="rounded-xl bg-surface-1 border border-border-subtle p-3.5 flex items-center gap-3">
          <Database className="w-4 h-4 text-emerald-400 shrink-0" />
          <div className="min-w-0">
            <div className="text-[11px] text-zinc-400">远端服务状态</div>
            <div className="text-xs font-mono text-zinc-100 truncate">
              {status?.reachable ? `Online · ${status.latency_ms} ms` : "Offline"}
            </div>
          </div>
        </div>
        <div className="rounded-xl bg-surface-1 border border-border-subtle p-3.5 flex items-center gap-3">
          <Cpu className="w-4 h-4 text-indigo-400 shrink-0" />
          <div className="min-w-0">
            <div className="text-[11px] text-zinc-400">Observer (Queue / VikingDB)</div>
            <div className="text-xs font-mono text-zinc-100 truncate">
              {queueLabel} / {vikingdbLabel}
            </div>
          </div>
        </div>
        <div className="rounded-xl bg-surface-1 border border-border-subtle p-3.5 flex items-center gap-3">
          <Layers className="w-4 h-4 text-sky-400 shrink-0" />
          <div className="min-w-0">
            <div className="text-[11px] text-zinc-400">当前列表累计 Token 消耗</div>
            <div className="text-xs font-mono text-zinc-100">
              {totalTokens.toLocaleString()} tokens（{events.length} 次事件）
            </div>
          </div>
        </div>
      </div>

      <div className="flex items-center gap-1.5">
        {AGENT_FILTERS.map((f) => (
          <button
            key={f.id}
            onClick={() => setAgentFilter(f.id)}
            className={`px-2.5 py-1 rounded-lg text-xs border transition-colors ${
              agentFilter === f.id
                ? "bg-indigo-500/15 text-indigo-300 border-indigo-500/30"
                : "bg-surface-1 text-zinc-400 border-border-subtle hover:text-zinc-200"
            }`}
          >
            {f.label}
          </button>
        ))}
      </div>

      <div className="rounded-xl border border-border-subtle bg-surface-1 divide-y divide-border-subtle">
        {events.map((evt) => {
          const isExpanded = expandedId === evt.id;
          const hits = hitsByEvent[evt.id] ?? [];
          const injectedCount = hits.filter((h) => h.injected).length;
          const skippedCount = hits.length - injectedCount;

          return (
            <div key={evt.id} className={isExpanded ? "bg-surface-2/40" : ""}>
              <button
                type="button"
                onClick={() => void toggleInspect(evt.id)}
                className="w-full text-left px-4 py-3 flex items-center justify-between gap-4 text-xs hover:bg-surface-2/50 transition-colors"
              >
                <div className="flex items-start gap-2.5 min-w-0">
                  <span className="mt-0.5 text-zinc-500 shrink-0">
                    {isExpanded ? (
                      <ChevronDown className="w-3.5 h-3.5 text-indigo-400" />
                    ) : (
                      <ChevronRight className="w-3.5 h-3.5" />
                    )}
                  </span>
                  <div className="min-w-0 space-y-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="px-1.5 py-0.5 rounded bg-indigo-500/15 text-indigo-300 font-mono text-[10px]">
                        {evt.agent_id}
                      </span>
                      <span
                        className={`px-1.5 py-0.5 rounded font-mono text-[10px] ${
                          evt.event_type === "recall" || evt.event_type === "hook_recall"
                            ? "bg-sky-500/15 text-sky-300"
                            : "bg-emerald-500/15 text-emerald-300"
                        }`}
                      >
                        {evt.event_type}
                      </span>
                      {evt.layer && (
                        <span className="px-1.5 py-0.5 rounded bg-surface-2 text-zinc-400 font-mono text-[10px] uppercase">
                          {evt.layer}
                        </span>
                      )}
                      {evt.blocked_by_privacy && (
                        <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-300 text-[10px]">
                          <ShieldOff className="w-3 h-3" />
                          隐私模式拦截
                        </span>
                      )}
                    </div>
                    <div className="text-zinc-200 truncate">
                      {evt.query ? `Query: ${evt.query}` : evt.target_uri ?? "—"}
                    </div>
                    {evt.target_uri && evt.query && (
                      <div className="font-mono text-[11px] text-zinc-500 truncate">
                        {evt.target_uri}
                      </div>
                    )}
                  </div>
                </div>

                <div className="shrink-0 text-right font-mono text-[11px] space-y-1">
                  <div className="text-zinc-300">{evt.tokens_used} tokens</div>
                  <div className="text-zinc-500">{evt.latency_ms} ms</div>
                </div>
              </button>

              {isExpanded && (
                <div className="px-4 pb-3.5 pt-1 pl-10 border-t border-border-subtle/60 bg-surface-0/60 space-y-2.5">
                  <div className="flex items-center justify-between gap-2 text-[11px]">
                    <div className="flex items-center gap-1.5 font-medium text-zinc-300">
                      <Search className="w-3.5 h-3.5 text-indigo-400" />
                      <span>Context Inspector · 召回候选明细</span>
                    </div>
                    {loadingHitsId === evt.id ? (
                      <span className="text-zinc-500 font-mono">加载候选明细中...</span>
                    ) : hits.length > 0 ? (
                      <div className="flex items-center gap-2 font-mono text-[10px]">
                        <span className="text-zinc-400">共 {hits.length} 条</span>
                        <span className="text-emerald-300">注入 {injectedCount}</span>
                        <span className="text-amber-300">拦截/跳过 {skippedCount}</span>
                      </div>
                    ) : null}
                  </div>

                  {loadingHitsId !== evt.id && hits.length === 0 ? (
                    <div className="text-[11px] text-zinc-500 py-2">
                      该事件未记录候选召回明细（如对话采集、启动索引或未命中任何候选）。
                    </div>
                  ) : (
                    <div className="rounded-lg border border-border-subtle bg-surface-1 divide-y divide-border-subtle/70">
                      {hits.map((hit) => {
                        const meta = REASON_META[hit.reason] ?? {
                          label: hit.reason || (hit.injected ? "已注入" : "已跳过"),
                          cls: hit.injected
                            ? "bg-emerald-500/15 text-emerald-300 border border-emerald-500/25"
                            : "bg-zinc-800 text-zinc-400 border border-border-subtle",
                        };
                        const pct = Math.max(0, Math.min(100, Math.round(hit.score * 100)));
                        return (
                          <div
                            key={hit.memory_uri}
                            className="px-3 py-2 flex items-center justify-between gap-3 text-[11px]"
                          >
                            <div className="flex items-center gap-2 min-w-0 flex-1">
                              {hit.injected ? (
                                <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
                              ) : (
                                <XCircle className="w-3.5 h-3.5 text-zinc-500 shrink-0" />
                              )}
                              <span
                                className={`font-mono truncate ${
                                  hit.injected ? "text-zinc-100" : "text-zinc-400"
                                }`}
                                title={hit.memory_uri}
                              >
                                {hit.memory_uri.replace(/^viking:\/\//, "")}
                              </span>
                            </div>

                            <div className="flex items-center gap-2.5 shrink-0">
                              <div className="flex items-center gap-1.5 w-24">
                                <div className="flex-1 h-1.5 rounded-full bg-surface-2 overflow-hidden">
                                  <div
                                    className={`h-full rounded-full ${
                                      hit.injected ? "bg-emerald-400" : "bg-zinc-500"
                                    }`}
                                    style={{ width: `${pct}%` }}
                                  />
                                </div>
                                <span className="font-mono text-[10px] text-zinc-300 w-8 text-right">
                                  {hit.score.toFixed(2)}
                                </span>
                              </div>
                              <span
                                className={`px-1.5 py-0.5 rounded text-[10px] whitespace-nowrap ${meta.cls}`}
                              >
                                {meta.label}
                              </span>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {events.length === 0 && (
          <div className="p-12 text-center text-xs text-zinc-500 flex flex-col items-center gap-2">
            <Activity className="w-5 h-5 text-zinc-600" />
            暂无事件记录
          </div>
        )}
      </div>
    </div>
  );
}
