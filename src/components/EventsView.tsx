import { useEffect, useState } from "react";
import { Activity, RefreshCw, ShieldOff } from "lucide-react";
import { api } from "../lib/tauri";
import type { RecallEvent } from "../types/openviking";

const AGENT_FILTERS = [
  { id: "", label: "全部 Agent" },
  { id: "claude-code", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "pi" },
  { id: "antigravity", label: "Antigravity" },
];

export function EventsView() {
  const [agentFilter, setAgentFilter] = useState("");
  const [events, setEvents] = useState<RecallEvent[]>([]);

  const load = () => {
    api.listRecallEvents(agentFilter || undefined).then(setEvents);
  };

  useEffect(() => {
    load();
  }, [agentFilter]);

  return (
    <div className="h-full flex flex-col p-6 space-y-4 overflow-hidden">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-zinc-100">实时召回与捕获事件流</h1>
          <p className="text-xs text-zinc-400 mt-0.5">
            追踪每个 Agent 通过 <code className="font-mono text-indigo-300">ov-bridge</code> 触发的 Recall / Capture、命中层级与 Token 消耗。
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

      <div className="flex-1 overflow-y-auto rounded-xl border border-border-subtle bg-surface-1 divide-y divide-border-subtle">
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
