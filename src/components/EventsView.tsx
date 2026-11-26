import { useEffect, useState } from "react";
import { Activity, Cpu, Database, Layers, RefreshCw, ShieldOff } from "lucide-react";
import { api } from "../lib/tauri";
import type { AgentStatus, RecallEvent, ServerStatus } from "../types/openviking";
import { MemoryFlowCanvas } from "./MemoryFlowCanvas";

const AGENT_FILTERS = [
  { id: "", label: "全部 Agent" },
  { id: "claude-code", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "pi" },
  { id: "antigravity", label: "Antigravity" },
];

interface EventsViewProps {
  agents: AgentStatus[];
  captureEnabled: boolean;
}

export function EventsView({ agents, captureEnabled }: EventsViewProps) {
  const [agentFilter, setAgentFilter] = useState("");
  const [events, setEvents] = useState<RecallEvent[]>([]);
  const [status, setStatus] = useState<ServerStatus | null>(null);

  const load = () => {
    void api.listRecallEvents(agentFilter || undefined).then(setEvents);
    void api.getServerStatus().then(setStatus);
  };

  useEffect(() => {
    load();
    const timer = window.setInterval(load, 5000);
    return () => window.clearInterval(timer);
  }, [agentFilter]);

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
    <div className="h-full flex flex-col p-6 space-y-4 overflow-y-auto">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-zinc-100">实时召回事件流与远端观测</h1>
          <p className="text-xs text-zinc-400 mt-0.5">
            追踪每个 Agent 通过 <code className="font-mono text-indigo-300">ov-bridge</code> 触发的 Recall / Capture、命中层级与远端 Observer 运行状态。
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
        {events.map((evt) => (
          <div key={evt.id} className="px-4 py-3 flex items-center justify-between gap-4 text-xs">
            <div className="min-w-0 space-y-1">
              <div className="flex items-center gap-2">
                <span className="px-1.5 py-0.5 rounded bg-indigo-500/15 text-indigo-300 font-mono text-[10px]">
                  {evt.agent_id}
                </span>
                <span
                  className={`px-1.5 py-0.5 rounded font-mono text-[10px] ${
                    evt.event_type === "recall"
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
                <div className="font-mono text-[11px] text-zinc-500 truncate">{evt.target_uri}</div>
              )}
            </div>

            <div className="shrink-0 text-right font-mono text-[11px] space-y-1">
              <div className="text-zinc-300">{evt.tokens_used} tokens</div>
              <div className="text-zinc-500">{evt.latency_ms} ms</div>
            </div>
          </div>
        ))}
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
