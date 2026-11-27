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
  onOpenPalette,
}: SidebarProps) {
  const isMac =
    typeof navigator !== "undefined" &&
    /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent);
  const shortcutLabel = isMac ? "⌘K" : "Ctrl+K";
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
    <aside className="w-60 shrink-0 bg-surface-1 border-r border-border-subtle flex flex-col justify-between select-none">
      <div className="p-3 space-y-4">
        <div className="flex items-center justify-between px-1.5">
          <div className="flex items-center gap-2">
            <div className="w-6 h-6 rounded-md bg-indigo-500/20 border border-indigo-400/30 flex items-center justify-center text-indigo-400 font-bold text-xs">
              OV
            </div>
            <span className="font-semibold text-sm tracking-tight text-zinc-100">
              OpenViking
            </span>
          </div>
          <span
            className={`inline-flex items-center gap-1.5 text-[11px] px-2 py-0.5 rounded-full border ${
              serverStatus?.reachable
                ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/20"
                : "bg-rose-500/10 text-rose-400 border-rose-500/20"
            }`}
          >
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                serverStatus?.reachable ? "bg-emerald-400" : "bg-rose-400"
              }`}
            />
            {serverStatus?.reachable ? `${serverStatus.latency_ms}ms` : "离线"}
          </span>
        </div>

        <button
          onClick={onOpenPalette}
          className="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-400 transition-colors"
        >
          <span className="flex items-center gap-2">
            <Search className="w-3.5 h-3.5" />
            快速跳转 / 搜索...
          </span>
          <kbd className="px-1.5 py-0.5 rounded bg-surface-0 text-[10px] text-zinc-500 font-mono">
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
              className="text-zinc-400 hover:text-indigo-300"
              title="管理远端实例"
            >
              <Settings2 className="w-3.5 h-3.5" />
            </button>
          </div>
          <select
            value={profiles?.active ?? "default"}
            onChange={(e) => onSwitchProfile(e.target.value)}
            className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 text-xs text-zinc-200 focus:outline-none focus:border-indigo-500/50"
          >
            {profiles &&
              Object.values(profiles.profiles).map((p) => (
                <option key={p.name} value={p.name}>
                  {p.label}
                </option>
              ))}
          </select>
        </div>

        <nav className="space-y-0.5">
          {navItems.map((item) => {
            const Icon = item.icon;
            const active = activeTab === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onSelectTab(item.id)}
                className={`w-full flex items-center justify-between px-2.5 py-2 rounded-lg text-xs font-medium transition-colors ${
                  active
                    ? "bg-indigo-500/15 text-indigo-300 border border-indigo-500/25"
                    : "text-zinc-400 hover:text-zinc-200 hover:bg-surface-2 border border-transparent"
                }`}
              >
                <span className="flex items-center gap-2.5">
                  <Icon className="w-4 h-4" />
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

      <div className="p-3 border-t border-border-subtle">
        <button
          onClick={onTogglePrivacy}
          className={`w-full flex items-center justify-between px-3 py-2 rounded-lg border text-xs transition-colors ${
            captureEnabled
              ? "bg-surface-2 border-border-subtle text-zinc-300 hover:bg-surface-3"
              : "bg-amber-500/15 border-amber-500/30 text-amber-300"
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
