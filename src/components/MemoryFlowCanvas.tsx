import { useState } from "react";
import { Cloud, Cpu, ShieldAlert, ShieldCheck, Sparkles } from "lucide-react";
import type { AgentId, AgentStatus, RecallEvent } from "../types/openviking";

interface MemoryFlowCanvasProps {
  agents: AgentStatus[];
  events: RecallEvent[];
  captureEnabled: boolean;
  serverReachable: boolean;
}

const AGENT_NODES: Array<{ id: AgentId; label: string; y: number }> = [
  { id: "claude-code", label: "Claude Code", y: 34 },
  { id: "codex", label: "Codex", y: 84 },
  { id: "pi", label: "pi Agent", y: 134 },
  { id: "antigravity", label: "Antigravity", y: 184 },
];

export function MemoryFlowCanvas({
  agents,
  events,
  captureEnabled,
  serverReachable,
}: MemoryFlowCanvasProps) {
  const [simMode, setSimMode] = useState<"live" | "capture" | "recall" | "privacy">("live");

  const latestEvent = events[0];
  const activeAgentId =
    simMode === "live"
      ? (latestEvent?.agent_id as AgentId | undefined) ?? "claude-code"
      : simMode === "capture"
        ? "codex"
        : simMode === "recall"
          ? "antigravity"
          : "claude-code";

  const activeDirection: "capture" | "recall" | "blocked" =
    simMode === "privacy" || (!captureEnabled && latestEvent?.event_type === "capture")
      ? "blocked"
      : simMode === "capture"
        ? "capture"
        : simMode === "recall"
          ? "recall"
          : latestEvent?.blocked_by_privacy
            ? "blocked"
            : latestEvent?.event_type === "capture" || latestEvent?.event_type === "hook_stop"
              ? "capture"
              : "recall";

  const connectedSet = new Set(agents.filter((a) => a.connected).map((a) => a.agent_id));

  const hubX = 340;
  const hubY = 109;
  const rightX = 610;
  const rightY = 109;

  return (
    <div className="rounded-xl bg-surface-1 border border-border-subtle p-4 space-y-3 overflow-hidden">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Sparkles className="w-3.5 h-3.5 text-indigo-400" />
          <span className="text-xs font-semibold text-zinc-100">实时记忆信息流拓扑</span>
          <span className="text-[11px] font-mono text-zinc-500">
            {activeDirection === "capture"
              ? "Capture 采集写入中（绿）"
              : activeDirection === "blocked"
                ? "隐私模式已拦截写入（琥珀）"
                : "Recall 记忆召回注入中（靛蓝）"}
          </span>
        </div>

        <div className="flex items-center gap-1 text-[10px]">
          {(
            [
              { id: "live", label: "实时" },
              { id: "capture", label: "演示采集" },
              { id: "recall", label: "演示召回" },
              { id: "privacy", label: "演示拦截" },
            ] as const
          ).map((m) => (
            <button
              key={m.id}
              onClick={() => setSimMode(m.id)}
              className={`px-2 py-0.5 rounded border transition-colors ${
                simMode === m.id
                  ? "bg-indigo-500/20 text-indigo-200 border-indigo-500/40"
                  : "bg-surface-2 text-zinc-400 border-border-subtle hover:text-zinc-200"
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>
      </div>

      {/* Hub-and-Spoke SVG Animated Beam Canvas */}
      <div className="relative w-full h-[218px] bg-surface-0/70 rounded-lg border border-border-subtle overflow-hidden">
        <svg
          viewBox="0 0 680 218"
          className="w-full h-full"
          aria-hidden="true"
          preserveAspectRatio="xMidYMid meet"
        >
          <defs>
            <linearGradient id="beamCapture" x1="0%" y1="0%" x2="100%" y2="0%">
              <stop offset="0%" stopColor="#10b981" stopOpacity="0.1" />
              <stop offset="50%" stopColor="#34d399" stopOpacity="1" />
              <stop offset="100%" stopColor="#10b981" stopOpacity="0.2" />
            </linearGradient>
            <linearGradient id="beamRecall" x1="100%" y1="0%" x2="0%" y2="0%">
              <stop offset="0%" stopColor="#6366f1" stopOpacity="0.1" />
              <stop offset="50%" stopColor="#818cf8" stopOpacity="1" />
              <stop offset="100%" stopColor="#38bdf8" stopOpacity="0.2" />
            </linearGradient>
            <filter id="glow" x="-40%" y="-40%" width="180%" height="180%">
              <feGaussianBlur stdDeviation="2.5" result="blur" />
              <feMerge>
                <feMergeNode in="blur" />
                <feMergeNode in="SourceGraphic" />
              </feMerge>
            </filter>
          </defs>

          {/* Left Bezier curves: Agent -> ov-bridge Hub */}
          {AGENT_NODES.map((node) => {
            const isConnected = connectedSet.has(node.id) || simMode !== "live";
            const isActive = activeAgentId === node.id;
            const pathId = `path-${node.id}`;
            const d = `M 148 ${node.y} C 235 ${node.y}, 255 ${hubY}, ${hubX - 42} ${hubY}`;
            const revD = `M ${hubX - 42} ${hubY} C 255 ${hubY}, 235 ${node.y}, 148 ${node.y}`;

            return (
              <g key={node.id}>
                <path
                  id={pathId}
                  d={d}
                  fill="none"
                  stroke={
                    isActive
                      ? activeDirection === "capture"
                        ? "rgba(52, 211, 153, 0.45)"
                        : activeDirection === "blocked"
                          ? "rgba(251, 191, 36, 0.45)"
                          : "rgba(129, 140, 248, 0.45)"
                      : isConnected
                        ? "rgba(255,255,255,0.12)"
                        : "rgba(255,255,255,0.05)"
                  }
                  strokeWidth={isActive ? 2 : 1.2}
                  strokeDasharray={isConnected ? undefined : "4 4"}
                />

                {isActive && activeDirection === "capture" && (
                  <circle r="4" fill="#34d399" filter="url(#glow)">
                    <animateMotion dur="1.5s" repeatCount="indefinite" path={d} />
                  </circle>
                )}

                {isActive && activeDirection === "recall" && (
                  <circle r="4" fill="#818cf8" filter="url(#glow)">
                    <animateMotion dur="1.5s" repeatCount="indefinite" path={revD} />
                  </circle>
                )}

                {isActive && activeDirection === "blocked" && (
                  <circle r="4" fill="#fbbf24" filter="url(#glow)">
                    <animateMotion
                      dur="1.4s"
                      repeatCount="indefinite"
                      keyPoints="0;0.88;0.78"
                      keyTimes="0;0.7;1"
                      calcMode="linear"
                      path={d}
                    />
                  </circle>
                )}

                {/* Agent Node Pill */}
                <g transform={`translate(18, ${node.y - 16})`}>
                  <rect
                    width="130"
                    height="32"
                    rx="8"
                    fill={isActive ? "#171c2b" : "#111318"}
                    stroke={
                      isActive
                        ? activeDirection === "capture"
                          ? "#34d399"
                          : activeDirection === "blocked"
                            ? "#fbbf24"
                            : "#818cf8"
                        : "rgba(255,255,255,0.12)"
                    }
                    strokeWidth={isActive ? 1.4 : 1}
                  />
                  <circle
                    cx="14"
                    cy="16"
                    r="3.5"
                    fill={isConnected ? "#34d399" : "#52525b"}
                  />
                  <text
                    x="26"
                    y="20"
                    fill={isActive ? "#f4f4f5" : "#a1a1aa"}
                    fontSize="11"
                    fontFamily="Inter, sans-serif"
                    fontWeight={isActive ? "600" : "400"}
                  >
                    {node.label}
                  </text>
                </g>
              </g>
            );
          })}

          {/* Right Link: ov-bridge Hub <-> Remote OpenViking */}
          {(() => {
            const dRight = `M ${hubX + 42} ${hubY} L ${rightX - 56} ${rightY}`;
            const dRightRev = `M ${rightX - 56} ${rightY} L ${hubX + 42} ${hubY}`;
            return (
              <g>
                <path
                  d={dRight}
                  fill="none"
                  stroke={
                    activeDirection === "capture"
                      ? "url(#beamCapture)"
                      : activeDirection === "recall"
                        ? "url(#beamRecall)"
                        : "rgba(255,255,255,0.1)"
                  }
                  strokeWidth={activeDirection === "blocked" ? 1.2 : 2.2}
                  strokeDasharray={activeDirection === "blocked" ? "5 5" : undefined}
                />
                {activeDirection === "capture" && (
                  <circle r="4.5" fill="#34d399" filter="url(#glow)">
                    <animateMotion dur="1.3s" begin="0.3s" repeatCount="indefinite" path={dRight} />
                  </circle>
                )}
                {activeDirection === "recall" && (
                  <circle r="4.5" fill="#38bdf8" filter="url(#glow)">
                    <animateMotion dur="1.3s" repeatCount="indefinite" path={dRightRev} />
                  </circle>
                )}
              </g>
            );
          })()}

          {/* Center Hub: ov-bridge */}
          <g transform={`translate(${hubX - 42}, ${hubY - 32})`}>
            <rect
              width="84"
              height="64"
              rx="14"
              fill="#151926"
              stroke={activeDirection === "blocked" ? "#fbbf24" : "#6366f1"}
              strokeWidth="1.5"
            />
            <text
              x="42"
              y="28"
              textAnchor="middle"
              fill="#e4e4e7"
              fontSize="11"
              fontWeight="600"
              fontFamily="JetBrains Mono, monospace"
            >
              ov-bridge
            </text>
            <text
              x="42"
              y="46"
              textAnchor="middle"
              fill={activeDirection === "blocked" ? "#fbbf24" : "#818cf8"}
              fontSize="9.5"
              fontFamily="Inter, sans-serif"
            >
              {activeDirection === "blocked" ? "隐私拦截中" : "策略与预算门控"}
            </text>
          </g>

          {/* Right Node: Remote OpenViking */}
          <g transform={`translate(${rightX - 56}, ${rightY - 32})`}>
            <rect
              width="112"
              height="64"
              rx="14"
              fill="#111622"
              stroke={serverReachable ? "#10b981" : "#f43f5e"}
              strokeWidth="1.4"
            />
            <text
              x="56"
              y="28"
              textAnchor="middle"
              fill="#f4f4f5"
              fontSize="11"
              fontWeight="600"
            >
              OpenViking
            </text>
            <text
              x="56"
              y="46"
              textAnchor="middle"
              fill={serverReachable ? "#34d399" : "#fb7185"}
              fontSize="9.5"
              fontFamily="JetBrains Mono, monospace"
            >
              {serverReachable ? "viking:// Online" : "Offline"}
            </text>
          </g>
        </svg>

        {/* Bottom-right Legend */}
        <div className="absolute bottom-2 right-3 flex items-center gap-3 text-[10px] text-zinc-400 bg-surface-1/80 backdrop-blur px-2.5 py-1 rounded-md border border-border-subtle">
          <span className="inline-flex items-center gap-1">
            <Cpu className="w-3 h-3 text-emerald-400" />
            采集写入
          </span>
          <span className="inline-flex items-center gap-1">
            <Cloud className="w-3 h-3 text-indigo-400" />
            召回注入
          </span>
          <span className="inline-flex items-center gap-1">
            {captureEnabled ? (
              <ShieldCheck className="w-3 h-3 text-emerald-400" />
            ) : (
              <ShieldAlert className="w-3 h-3 text-amber-400" />
            )}
            {captureEnabled ? "捕获开启" : "隐私只读"}
          </span>
        </div>
      </div>
    </div>
  );
}
