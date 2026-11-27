import { useEffect, useState } from "react";
import { Search } from "lucide-react";
import type { NavTab } from "./Sidebar";

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  onSelectTab: (tab: NavTab) => void;
  onTriggerDreaming: () => void;
  onTogglePrivacy: () => void;
}

export function CommandPalette({
  open,
  onClose,
  onSelectTab,
  onTriggerDreaming,
  onTogglePrivacy,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);

  const actions = [
    { id: "agents", title: "切换到：本地 Agent 接入（Claude / Codex / pi / Antigravity）", run: () => onSelectTab("agents") },
    { id: "explorer", title: "切换到：viking:// 记忆浏览器（L0 / L1 / L2 编辑）", run: () => onSelectTab("explorer") },
    { id: "dreaming", title: "切换到：Dreaming 审核台（PR Diff 审批）", run: () => onSelectTab("dreaming") },
    { id: "events", title: "切换到：实时召回/捕获事件流", run: () => onSelectTab("events") },
    { id: "policy", title: "切换到：记忆策略中心（路由 / Token 预算 / 定时整理）", run: () => onSelectTab("policy") },
    { id: "run-dream", title: "立即执行一次 Dreaming 后台整理", run: onTriggerDreaming },
    { id: "privacy", title: "切换隐私模式（暂停/恢复自动捕获）", run: onTogglePrivacy },
  ].filter((a) => a.title.toLowerCase().includes(query.toLowerCase()));

  useEffect(() => {
    setActiveIndex(0);
  }, [query, open]);

  useEffect(() => {
    if (!open) {
      setQuery("");
      return;
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.isComposing || e.keyCode === 229) return;
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (e.key === "ArrowDown" && actions.length > 0) {
      e.preventDefault();
      setActiveIndex((prev) => (prev + 1) % actions.length);
    } else if (e.key === "ArrowUp" && actions.length > 0) {
      e.preventDefault();
      setActiveIndex((prev) => (prev - 1 + actions.length) % actions.length);
    } else if (e.key === "Enter" && actions.length > 0) {
      e.preventDefault();
      const chosen = actions[Math.min(activeIndex, actions.length - 1)];
      if (chosen) {
        chosen.run();
        onClose();
      }
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-start justify-center pt-24"
      onClick={onClose}
    >
      <div
        className="w-full max-w-lg bg-surface-1 border border-border-strong rounded-xl shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 px-4 py-3 border-b border-border-subtle">
          <Search className="w-4 h-4 text-zinc-400" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleInputKeyDown}
            placeholder="输入命令或页面名称..."
            className="w-full bg-transparent text-sm text-zinc-100 placeholder:text-zinc-500 focus:outline-none"
          />
          <kbd className="text-[10px] font-mono text-zinc-500 px-1.5 py-0.5 rounded bg-surface-2">
            ESC
          </kbd>
        </div>
        <div className="max-h-72 overflow-y-auto p-1.5">
          {actions.map((item, idx) => (
            <button
              key={item.id}
              onMouseEnter={() => setActiveIndex(idx)}
              onFocus={() => setActiveIndex(idx)}
              onClick={() => {
                item.run();
                onClose();
              }}
              className={`w-full text-left px-3 py-2 rounded-lg text-xs transition-colors ${
                idx === activeIndex
                  ? "bg-indigo-500/15 text-indigo-200"
                  : "text-zinc-200 hover:bg-indigo-500/15 hover:text-indigo-200"
              }`}
            >
              {item.title}
            </button>
          ))}
          {actions.length === 0 && (
            <div className="px-3 py-6 text-center text-xs text-zinc-500">
              未找到匹配命令
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
