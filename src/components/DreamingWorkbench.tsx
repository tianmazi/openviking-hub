import { useEffect, useMemo, useState } from "react";
import {
  Check,
  CheckCheck,
  Edit3,
  History,
  Pin,
  Play,
  RotateCcw,
  Sparkles,
  X,
} from "lucide-react";
import type {
  ApplyReport,
  ChangeAction,
  DreamingRun,
  PendingMemoryChange,
  ReviewStatus,
} from "../types/openviking";

interface DreamingWorkbenchProps {
  changes: PendingMemoryChange[];
  runs: DreamingRun[];
  onTriggerDreaming: () => Promise<void>;
  onUpdateDecision: (
    id: string,
    status: ReviewStatus,
    editedAfterContent?: string,
  ) => Promise<void>;
  onApplyAccepted: () => Promise<ApplyReport>;
  onRollbackRun: (runId: string) => Promise<void>;
  onPinUri: (uri: string) => Promise<void>;
}

const ACTION_LABELS: Record<ChangeAction, string> = {
  merge: "冲突合并",
  move_from_inbox: "Inbox 分拣",
  promote_to_index: "晋升索引",
  demote_from_index: "下沉目录",
  archive: "冷记忆归档",
  wiki_compile: "Wiki 编译",
};

export function DreamingWorkbench({
  changes,
  runs,
  onTriggerDreaming,
  onUpdateDecision,
  onApplyAccepted,
  onRollbackRun,
  onPinUri,
}: DreamingWorkbenchProps) {
  const [filterAction, setFilterAction] = useState<string>("all");
  const [selectedId, setSelectedId] = useState<string | null>(changes[0]?.id ?? null);
  const [detailTab, setDetailTab] = useState<"diff" | "evidence" | "layers">("diff");
  const [editing, setEditing] = useState(false);
  const [editedText, setEditedText] = useState("");
  const [showHistory, setShowHistory] = useState(false);
  const [busy, setBusy] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);

  const visibleChanges = useMemo(
    () =>
      changes.filter((c) => {
        if (c.status === "applied") return false;
        if (filterAction === "all") return true;
        return c.action === filterAction;
      }),
    [changes, filterAction],
  );

  const selected = useMemo(
    () => visibleChanges.find((c) => c.id === selectedId) ?? visibleChanges[0] ?? null,
    [visibleChanges, selectedId],
  );

  const acceptedCount = useMemo(
    () => changes.filter((c) => c.status === "accepted").length,
    [changes],
  );

  useEffect(() => {
    if (selected) {
      setEditedText(selected.after_content ?? "");
      setEditing(false);
    }
  }, [selected?.id]);

  // Linear / GitHub PR Review keyboard shortcuts: J/K/A/X/E/Cmd+Enter
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (
        e.target instanceof HTMLTextAreaElement ||
        e.target instanceof HTMLInputElement
      ) {
        if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && selected) {
          e.preventDefault();
          void onUpdateDecision(selected.id, "accepted", editedText);
          setEditing(false);
        }
        return;
      }
      if (!selected) return;
      const idx = visibleChanges.findIndex((c) => c.id === selected.id);
      if (e.key === "j" || e.key === "J") {
        const next = visibleChanges[Math.min(visibleChanges.length - 1, idx + 1)];
        if (next) setSelectedId(next.id);
      } else if (e.key === "k" || e.key === "K") {
        const prev = visibleChanges[Math.max(0, idx - 1)];
        if (prev) setSelectedId(prev.id);
      } else if (e.key === "a" || e.key === "A") {
        void onUpdateDecision(selected.id, "accepted");
      } else if (e.key === "x" || e.key === "X") {
        void onUpdateDecision(selected.id, "rejected");
      } else if (e.key === "e" || e.key === "E") {
        e.preventDefault();
        setDetailTab("diff");
        setEditing(true);
      } else if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && acceptedCount > 0) {
        e.preventDefault();
        void handleApply();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selected, visibleChanges, editedText, acceptedCount]);

  const handleAcceptLowRisk = async () => {
    setBusy(true);
    try {
      for (const c of visibleChanges) {
        if (c.risk_level === "low" && c.status === "pending") {
          await onUpdateDecision(c.id, "accepted");
        }
      }
      setBanner("已批量暂存所有低风险项（Low Risk），点击右上角“提交已批准”才会写入远端。");
    } finally {
      setBusy(false);
    }
  };

  const handleApply = async () => {
    setBusy(true);
    setBanner(null);
    try {
      const report = await onApplyAccepted();
      setBanner(
        `提交完成：应用 ${report.applied_ids.length} 项，冲突拦截 ${report.conflict_ids.length} 项` +
          (report.snapshot_id ? `（Snapshot: ${report.snapshot_id}）` : ""),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* Top Workbench Header */}
      <div className="px-5 py-3.5 border-b border-border-subtle bg-surface-1 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-indigo-400" />
            <h1 className="text-sm font-semibold text-zinc-100">Dreaming 审核台</h1>
          </div>
          <span className="text-[11px] text-zinc-500 font-mono">
            快捷键: J/K 切换 · A 批准 · X 拒绝 · E 编辑终稿 · ⌘↵ 提交
          </span>
        </div>

        <div className="flex items-center gap-2">
          <button
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onTriggerDreaming();
              } finally {
                setBusy(false);
              }
            }}
            className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-200"
          >
            <Play className="w-3.5 h-3.5 text-indigo-400" />
            立即整理
          </button>

          <button
            onClick={() => setShowHistory((v) => !v)}
            className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-200"
          >
            <History className="w-3.5 h-3.5" />
            历史与回滚 ({runs.length})
          </button>

          <button
            disabled={busy}
            onClick={handleAcceptLowRisk}
            className="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-emerald-300"
          >
            <CheckCheck className="w-3.5 h-3.5" />
            批量批准安全项
          </button>

          <button
            disabled={busy || acceptedCount === 0}
            onClick={handleApply}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs font-medium text-white"
          >
            提交已批准 ({acceptedCount})
          </button>
        </div>
      </div>

      {/* Filter Bar */}
      <div className="px-5 py-2 border-b border-border-subtle bg-surface-1/60 flex items-center gap-1.5 overflow-x-auto">
        {[
          { id: "all", label: `全部 (${visibleChanges.length})` },
          { id: "merge", label: "冲突合并" },
          { id: "move_from_inbox", label: "Inbox 分拣" },
          { id: "archive", label: "冷记忆归档" },
          { id: "wiki_compile", label: "Wiki 编译" },
        ].map((f) => (
          <button
            key={f.id}
            onClick={() => setFilterAction(f.id)}
            className={`px-2.5 py-1 rounded-md text-[11px] border transition-colors ${
              filterAction === f.id
                ? "bg-indigo-500/15 text-indigo-300 border-indigo-500/30"
                : "bg-surface-2 text-zinc-400 border-border-subtle hover:text-zinc-200"
            }`}
          >
            {f.label}
          </button>
        ))}
      </div>

      {banner && (
        <div className="px-5 py-2 bg-indigo-500/10 border-b border-indigo-500/20 text-xs text-indigo-200">
          {banner}
        </div>
      )}

      {/* Optional Snapshot History & Rollback Drawer */}
      {showHistory && (
        <div className="px-5 py-3 bg-surface-2 border-b border-border-subtle space-y-2">
          <div className="text-xs font-semibold text-zinc-200">
            Dreaming 运行历史与 Snapshot 一键回滚
          </div>
          <div className="space-y-1.5 max-h-36 overflow-y-auto">
            {runs.map((r) => (
              <div
                key={r.id}
                className="flex items-center justify-between px-3 py-1.5 rounded-lg bg-surface-1 border border-border-subtle text-xs"
              >
                <div className="flex items-center gap-3 font-mono text-[11px]">
                  <span className="text-zinc-200">{r.id}</span>
                  <span className="text-zinc-500">{r.trigger_source}</span>
                  <span className="text-indigo-300">{r.status}</span>
                  {r.snapshot_id && <span className="text-zinc-400">snap: {r.snapshot_id}</span>}
                </div>
                {r.snapshot_id && r.status !== "rolled_back" && (
                  <button
                    onClick={() => onRollbackRun(r.id)}
                    className="inline-flex items-center gap-1 px-2 py-1 rounded bg-rose-500/15 hover:bg-rose-500/25 text-rose-300 text-[11px]"
                  >
                    <RotateCcw className="w-3 h-3" />
                    回滚快照
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Split Pane: Proposal Queue (Left) + Diff/Evidence Workspace (Right) */}
      <div className="flex-1 flex overflow-hidden">
        <div className="w-80 shrink-0 border-r border-border-subtle bg-surface-1/40 overflow-y-auto p-2 space-y-1.5">
          {visibleChanges.map((item) => {
            const isSelected = selected?.id === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setSelectedId(item.id)}
                className={`w-full text-left p-3 rounded-xl border transition-colors space-y-1.5 ${
                  isSelected
                    ? "bg-indigo-500/10 border-indigo-500/35"
                    : "bg-surface-1 border-border-subtle hover:border-border-strong"
                }`}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-surface-2 text-indigo-300">
                    {ACTION_LABELS[item.action]}
                  </span>
                  <span
                    className={`text-[10px] px-1.5 py-0.5 rounded-full ${
                      item.status === "accepted"
                        ? "bg-emerald-500/15 text-emerald-300"
                        : item.status === "rejected"
                          ? "bg-zinc-800 text-zinc-500"
                          : item.status === "conflict"
                            ? "bg-rose-500/20 text-rose-300"
                            : "bg-amber-500/15 text-amber-300"
                    }`}
                  >
                    {item.status === "accepted"
                      ? "已暂存批准"
                      : item.status === "rejected"
                        ? "已拒绝"
                        : item.status === "conflict"
                          ? "远端漂移冲突"
                          : "待审"}
                  </span>
                </div>
                <div className="font-mono text-[11px] text-zinc-200 truncate">
                  {item.target_uri.replace("viking://", "")}
                </div>
                <div className="text-[11px] text-zinc-400 line-clamp-2">{item.rationale}</div>
              </button>
            );
          })}
          {visibleChanges.length === 0 && (
            <div className="p-8 text-center text-xs text-zinc-500">
              当前队列已清空，点击“立即整理”扫描远端记忆。
            </div>
          )}
        </div>

        {/* Right Detail Workspace */}
        <div className="flex-1 flex flex-col bg-surface-0 overflow-hidden">
          {selected ? (
            <>
              <div className="px-5 py-3.5 border-b border-border-subtle space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <div className="font-mono text-xs text-zinc-100 truncate">
                    {selected.target_uri}
                  </div>
                  <div className="flex items-center gap-1.5">
                    {(["diff", "evidence", "layers"] as const).map((tab) => (
                      <button
                        key={tab}
                        onClick={() => setDetailTab(tab)}
                        className={`px-2.5 py-1 rounded-md text-[11px] border ${
                          detailTab === tab
                            ? "bg-indigo-500/15 text-indigo-300 border-indigo-500/30"
                            : "bg-surface-1 text-zinc-400 border-border-subtle"
                        }`}
                      >
                        {tab === "diff"
                          ? "Diff 对比"
                          : tab === "evidence"
                            ? "证据链"
                            : "L0/L1 影响预览"}
                      </button>
                    ))}
                  </div>
                </div>
                <p className="text-xs text-zinc-400">{selected.rationale}</p>
              </div>

              <div className="flex-1 overflow-y-auto p-5">
                {detailTab === "diff" && (
                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 h-full">
                    <div className="flex flex-col rounded-xl bg-surface-1 border border-rose-500/20 overflow-hidden">
                      <div className="px-3 py-2 bg-rose-500/10 border-b border-rose-500/20 text-[11px] font-mono text-rose-300">
                        Before（当前远端内容）
                      </div>
                      <pre className="flex-1 p-4 font-mono text-xs text-zinc-300 whitespace-pre-wrap overflow-y-auto">
                        {selected.before_content ?? "（无旧内容 / 新增操作）"}
                      </pre>
                    </div>

                    <div className="flex flex-col rounded-xl bg-surface-1 border border-emerald-500/20 overflow-hidden">
                      <div className="px-3 py-2 bg-emerald-500/10 border-b border-emerald-500/20 flex items-center justify-between">
                        <span className="text-[11px] font-mono text-emerald-300">
                          After（提案终稿 — {editing ? "编辑中" : "可按 E 直接修改"}）
                        </span>
                        <button
                          onClick={() => setEditing((v) => !v)}
                          className="inline-flex items-center gap-1 text-[11px] text-emerald-200 hover:underline"
                        >
                          <Edit3 className="w-3 h-3" />
                          {editing ? "预览" : "编辑终稿"}
                        </button>
                      </div>
                      {editing ? (
                        <textarea
                          autoFocus
                          value={editedText}
                          onChange={(e) => setEditedText(e.target.value)}
                          className="flex-1 p-4 bg-transparent font-mono text-xs text-zinc-100 focus:outline-none resize-none"
                        />
                      ) : (
                        <pre className="flex-1 p-4 font-mono text-xs text-zinc-200 whitespace-pre-wrap overflow-y-auto">
                          {editedText ||
                            (selected.destination_uri
                              ? `→ 移动/归档至: ${selected.destination_uri}`
                              : "（无文本写入）")}
                        </pre>
                      )}
                    </div>
                  </div>
                )}

                {detailTab === "evidence" && (
                  <div className="rounded-xl bg-surface-1 border border-border-subtle p-4 space-y-2">
                    <div className="text-xs font-semibold text-zinc-200">
                      决策证据链（本地 SQLite 召回统计与来源）
                    </div>
                    <pre className="font-mono text-xs text-zinc-300 whitespace-pre-wrap">
                      {(() => {
                        try {
                          return JSON.stringify(JSON.parse(selected.evidence_json), null, 2);
                        } catch {
                          return selected.evidence_json;
                        }
                      })()}
                    </pre>
                  </div>
                )}

                {detailTab === "layers" && (
                  <div className="rounded-xl bg-surface-1 border border-border-subtle p-4 space-y-3 text-xs">
                    <div className="font-semibold text-zinc-200">L0 / L1 分层摘要影响预览</div>
                    <div className="p-3 rounded-lg bg-surface-2 border border-border-subtle">
                      <div className="text-[11px] font-mono text-indigo-300 mb-1">
                        预计 L0 摘要 (~100 tokens)
                      </div>
                      <p className="text-zinc-300">
                        {(editedText || selected.rationale).slice(0, 180)}
                      </p>
                    </div>
                  </div>
                )}
              </div>

              {/* Bottom Action Bar */}
              <div className="px-5 py-3 border-t border-border-subtle bg-surface-1 flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => onUpdateDecision(selected.id, "rejected")}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-rose-500/20 hover:text-rose-300 border border-border-subtle text-xs text-zinc-300"
                  >
                    <X className="w-3.5 h-3.5" />
                    拒绝 (X)
                  </button>
                  <button
                    onClick={async () => {
                      await onPinUri(selected.target_uri);
                      await onUpdateDecision(selected.id, "rejected");
                      setBanner(`已将 ${selected.target_uri} 加入永久免归档保护列表。`);
                    }}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-300"
                  >
                    <Pin className="w-3.5 h-3.5 text-amber-400" />
                    永久保留 / 免打扰
                  </button>
                </div>

                <button
                  onClick={async () => {
                    await onUpdateDecision(
                      selected.id,
                      "accepted",
                      editing ? editedText : undefined,
                    );
                    setEditing(false);
                  }}
                  className="inline-flex items-center gap-1.5 px-4 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-xs font-medium text-white"
                >
                  <Check className="w-3.5 h-3.5" />
                  批准并暂存 (A)
                </button>
              </div>
            </>
          ) : (
            <div className="flex-1 flex items-center justify-center text-xs text-zinc-500">
              选择左侧变更提案查看 Diff 与审批
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
