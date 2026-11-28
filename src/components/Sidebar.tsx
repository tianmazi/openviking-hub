import {
  Activity,
  Bot,
  FolderTree,
  GitPullRequestDraft,
  Search,
  Server,
  Settings2,
  ShieldAlert,
  ShieldCheck,
  Sliders,
  Sparkles,
} from "lucide-react";
import type { ProfileStore, ServerStatus } from "../types/openviking";

export type NavTab = "agents" | "explorer" | "dreaming" | "events" | "policy";

interface SidebarProps {
  activeTab: NavTab;
  onSelectTab: (tab: NavTab) => void;
  serverStatus: ServerStatus | null;
  profiles: ProfileStore | null;
  onSwitchProfile: (name: string) => void;
  onManageProfiles: () => void;
  captureEnabled: boolean;
  onTogglePrivacy: () => void;
  pendingDreamCount: number;
  connectedAgentsCount: number;
  totalAgentsCount: number;
  maxRecallTokens: number;
  onOpenPalette: () => void;
}

export function Sidebar({
  activeTab,
  onSelectTab,
  serverStatus,
  profiles,
  onSwitchProfile,
  onManageProfiles,
  captureEnabled,
  onTogglePrivacy,
  pendingDreamCount,
  connectedAgentsCount,
  totalAgentsCount,
  maxRecallTokens,
  onOpenPalette,
}: SidebarProps) {
  const isMac =
    typeof navigator !== "undefined" &&
    /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent);
  const shortcutLabel = isMac ? "⌘K" : "Ctrl+K";

  // 3-state semantic top ambient rim: Privacy (amber) > Pending Dreaming (indigo) > Online (emerald) / Offline (rose)
  const rimGradient = !captureEnabled
    ? "from-amber-400/75 via-amber-500/25 to-transparent"
    : !serverStatus?.reachable
      ? "from-rose-400/70 via-rose-500/20 to-transparent"
      : pendingDreamCount > 0
        ? "from-indigo-400/80 via-violet-400/30 to-transparent"
        : "from-emerald-400/65 via-sky-400/25 to-transparent";

  const agentRatio =
    totalAgentsCount > 0 ? Math.max(12, Math.round((connectedAgentsCount / totalAgentsCount) * 100)) : 0;
  const vialLiquidColor = !captureEnabled
    ? "#f59e0b"
    : serverStatus?.reachable
      ? "#6366f1"
      : "#f43f5e";

  const navItems: Array<{
    id: NavTab;
    label: string;
    icon: typeof Bot;
    badge?: number;
  }> = [
    { id: "agents", label: "本地 Agent 接入", icon: Bot },
    { id: "explorer", label: "viking:// 记忆浏览器", icon: FolderTree },
    {
      id: "dreaming",
      label: "Dreaming 审核台",
      icon: GitPullRequestDraft,
      badge: pendingDreamCount > 0 ? pendingDreamCount : undefined,
    },
    { id: "events", label: "实时召回事件流", icon: Activity },
    { id: "policy", label: "记忆策略中心", icon: Sliders },
  ];

  return (
    <aside className="relative w-60 shrink-0 bg-surface-1/95 border-r border-border-subtle flex flex-col justify-between select-none">
      {/* Subtle 2px optical semantic top rim */}
      <div
        className={`pointer-events-none absolute inset-x-0 top-0 h-[2px] bg-gradient-to-r ${rimGradient}`}
      />

      <div className="p-3 space-y-4">
        <div className="flex items-center justify-between px-1.5 pt-0.5">
          <div className="flex items-center gap-2.5">
            <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-indigo-500/30 via-violet-500/20 to-sky-500/20 border border-indigo-400/35 shadow-facet flex items-center justify-center text-indigo-200 font-bold text-[11px] tracking-tight">
              OV
            </div>
            <div className="leading-tight">
              <div className="font-semibold text-sm tracking-tight text-zinc-100">
                OpenViking
              </div>
              <div className="text-[10px] font-mono text-zinc-500">Hub Control Plane</div>
            </div>
          </div>
          <span
            className={`inline-flex items-center gap-1.5 text-[11px] px-2 py-0.5 rounded-full border font-mono ${
              serverStatus?.reachable
                ? "bg-emerald-500/10 text-emerald-300 border-emerald-500/25"
                : "bg-rose-500/10 text-rose-300 border-rose-500/25"
            }`}
          >
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                serverStatus?.reachable
                  ? "bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.8)]"
                  : "bg-rose-400"
              }`}
            />
            {serverStatus?.reachable ? `${serverStatus.latency_ms}ms` : "离线"}
          </span>
        </div>

        <button
          onClick={onOpenPalette}
          className="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg bg-surface-2/90 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-400 hover:text-zinc-200 transition-colors"
        >
          <span className="flex items-center gap-2">
            <Search className="w-3.5 h-3.5 text-zinc-500" />
            快速跳转 / 搜索...
          </span>
          <kbd className="px-1.5 py-0.5 rounded bg-surface-0 text-[10px] text-zinc-400 font-mono">
            {shortcutLabel}
          </kbd>
        </button>

        <div className="space-y-1.5">
          <div className="px-1.5 flex items-center justify-between text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
            <span className="flex items-center gap-1.5">
              <Server className="w-3 h-3" />
              远端实例 Profile
            </span>
            <button
              onClick={onManageProfiles}
              className="text-zinc-400 hover:text-indigo-300 transition-colors"
              title="管理远端实例"
            >
              <Settings2 className="w-3.5 h-3.5" />
            </button>
          </div>
          <select
            value={profiles?.active ?? "default"}
            onChange={(e) => onSwitchProfile(e.target.value)}
            className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 text-xs text-zinc-200 focus:outline-none focus:border-indigo-500/50 transition-colors"
          >
            {profiles &&
              Object.values(profiles.profiles).map((p) => (
                <option key={p.name} value={p.name}>
                  {p.label}
                </option>
              ))}
          </select>
        </div>

        <nav className="space-y-1">
          {navItems.map((item) => {
            const Icon = item.icon;
            const active = activeTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onSelectTab(item.id)}
                className={`relative w-full flex items-center justify-between px-2.5 py-2 rounded-lg text-xs font-medium transition-all ${
                  active
                    ? "bg-indigo-500/15 text-indigo-200 border border-indigo-400/25 shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]"
                    : "text-zinc-400 hover:text-zinc-200 hover:bg-surface-2/80 border border-transparent"
                }`}
              >
                {active && (
                  <span className="absolute left-0 top-1.5 bottom-1.5 w-[2.5px] rounded-r-full bg-indigo-400 shadow-[0_0_8px_rgba(129,140,248,0.85)]" />
                )}
                <span className="flex items-center gap-2.5">
                  <Icon className={`w-4 h-4 ${active ? "text-indigo-300" : "text-zinc-500"}`} />
                  {item.label}
                </span>
                {item.badge !== undefined && (
                  <span className="px-1.5 py-0.5 text-[10px] font-mono rounded-full bg-amber-500/20 text-amber-300 border border-amber-500/30">
                    {item.badge}
                  </span>
                )}
              </button>
            );
          })}
        </nav>
      </div>

      <div className="p-3 border-t border-border-subtle space-y-2.5">
        {/* Compact Optical Memory Status Vial */}
        <button
          type="button"
          onClick={() => onSelectTab("events")}
          className="w-full text-left p-2.5 rounded-xl bg-surface-2/70 hover:bg-surface-2 border border-border-subtle transition-colors space-y-2"
          title="点击打开实时召回事件流与 Context Inspector"
        >
          <div className="flex items-center justify-between text-[10px]">
            <span className="inline-flex items-center gap-1 text-zinc-400 font-medium">
              <Sparkles className="w-3 h-3 text-indigo-400" />
              记忆控制面状态
            </span>
            <span className="font-mono text-zinc-300">
              {connectedAgentsCount}/{totalAgentsCount || 4} Agents
            </span>
          </div>
          <div className="relative h-3 rounded-full ov-vial-glass border border-white/10 overflow-hidden">
            <svg
              className="absolute inset-0 w-full h-full"
              viewBox="0 0 200 12"
              preserveAspectRatio="none"
            >
              <defs>
                <clipPath id="ov-vial-clip">
                  <rect x="0" y="0" width={Math.max(12, agentRatio * 2)} height="12" rx="6" />
                </clipPath>
              </defs>
              <g clipPath="url(#ov-vial-clip)">
                <rect
                  x="0"
                  y="0"
                  width="200"
                  height="12"
                  fill={vialLiquidColor}
                  fillOpacity="0.45"
                />
                <path
                  className={serverStatus?.reachable ? "ov-vial-wave" : ""}
                  d="M0 5 Q 7 2, 14 5 T 28 5 T 42 5 T 56 5 T 70 5 T 84 5 T 98 5 T 112 5 T 126 5 T 140 5 T 154 5 T 168 5 T 182 5 T 196 5 T 210 5 T 224 5 V 12 H 0 Z"
                  fill={vialLiquidColor}
                  fillOpacity="0.65"
                />
              </g>
            </svg>
          </div>
          <div className="flex items-center justify-between font-mono text-[10px] text-zinc-500">
            <span>预算 {maxRecallTokens} tok</span>
            <span>{pendingDreamCount > 0 ? `${pendingDreamCount} 待审提案` : "队列清空"}</span>
          </div>
        </button>

        <button
          onClick={onTogglePrivacy}
          className={`w-full flex items-center justify-between px-3 py-2 rounded-lg border text-xs transition-colors ${
            captureEnabled
              ? "bg-surface-2 border-border-subtle text-zinc-300 hover:bg-surface-3"
              : "bg-amber-500/15 border-amber-500/30 text-amber-200 shadow-[inset_0_1px_0_rgba(251,191,36,0.18)]"
          }`}
        >
          <span className="flex items-center gap-2 font-medium">
            {captureEnabled ? (
              <ShieldCheck className="w-4 h-4 text-emerald-400" />
            ) : (
              <ShieldAlert className="w-4 h-4 text-amber-400" />
            )}
            {captureEnabled ? "自动捕获已开启" : "隐私模式（只读）"}
          </span>
          <span className="text-[10px] text-zinc-500">切换</span>
        </button>
      </div>
    </aside>
  );
}
