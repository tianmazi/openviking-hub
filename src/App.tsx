import { useEffect, useState } from "react";
import { AlertTriangle, WifiOff } from "lucide-react";
import { api } from "./lib/tauri";
import type {
  AgentId,
  AgentStatus,
  DreamingRun,
  PendingMemoryChange,
  PolicyConfig,
  ProfileStore,
  ReviewStatus,
  ServerStatus,
  WslDistroInfo,
} from "./types/openviking";
import { Sidebar, type NavTab } from "./components/Sidebar";
import { CommandPalette } from "./components/CommandPalette";
import { ProfileModal } from "./components/ProfileModal";
import { ToastStack, type ToastItem } from "./components/ToastStack";
import { AgentsView } from "./components/AgentsView";
import { ExplorerView } from "./components/ExplorerView";
import { EventsView } from "./components/EventsView";
import { PolicyView } from "./components/PolicyView";
import { DreamingWorkbench } from "./components/DreamingWorkbench";

export default function App() {
  const [activeTab, setActiveTab] = useState<NavTab>("agents");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [profileModalOpen, setProfileModalOpen] = useState(false);
  const [serverStatus, setServerStatus] = useState<ServerStatus | null>(null);
  const [profiles, setProfiles] = useState<ProfileStore | null>(null);
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [wslDistros, setWslDistros] = useState<WslDistroInfo[]>([]);
  const [policy, setPolicy] = useState<PolicyConfig | null>(null);
  const [changes, setChanges] = useState<PendingMemoryChange[]>([]);
  const [runs, setRuns] = useState<DreamingRun[]>([]);
  const [switchingProfile, setSwitchingProfile] = useState(false);
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const pushToast = (kind: ToastItem["kind"], message: string) => {
    const id = `${Date.now()}-${Math.random()}`;
    setToasts((prev) => [...prev.slice(-3), { id, kind, message }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4500);
  };

  const bootstrap = async () => {
    try {
      const [st, profs, ags, wsl, pol, chgs, rns] = await Promise.all([
        api.getServerStatus(),
        api.getProfiles(),
        api.detectAgents(),
        api.detectWsl(),
        api.getPolicy(),
        api.listPendingChanges(),
        api.listDreamingRuns(),
      ]);
      setServerStatus(st);
      setProfiles(profs);
      setAgents(ags);
      setWslDistros(wsl);
      setPolicy(pol);
      setChanges(chgs);
      setRuns(rns);
    } catch (e) {
      pushToast("error", `初始化状态失败：${String(e)}`);
    }
  };

  useEffect(() => {
    void bootstrap();
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const handleSaveProfiles = async (next: ProfileStore) => {
    setSwitchingProfile(true);
    setChanges([]);
    setRuns([]);
    try {
      await api.saveProfiles(next);
      setProfiles(next);
      const [st, chgs, rns] = await Promise.all([
        api.getServerStatus(),
        api.listPendingChanges(),
        api.listDreamingRuns(),
      ]);
      setServerStatus(st);
      setChanges(chgs);
      setRuns(rns);
      pushToast(
        st.reachable ? "success" : "info",
        st.reachable
          ? `已切换至 ${next.profiles[next.active]?.label ?? next.active}（延迟 ${st.latency_ms}ms）`
          : `已保存 Profile，但目标服务暂不可达`,
      );
    } finally {
      setSwitchingProfile(false);
    }
  };

  const handleSwitchProfile = async (name: string) => {
    if (!profiles) return;
    await handleSaveProfiles({ ...profiles, active: name });
  };

  const handleTogglePrivacy = async () => {
    if (!policy) return;
    const nextEnabled = !policy.global.capture_enabled;
    const updated = await api.togglePrivacyMode(nextEnabled);
    setPolicy(updated);
    pushToast(
      "info",
      nextEnabled ? "已恢复自动记忆捕获" : "已切换至隐私模式（只读，拦截所有自动写入）",
    );
  };

  const handleConnectAgent = async (id: AgentId, writeRules: boolean, projectDir?: string) => {
    const updated = await api.connectAgent(id, writeRules, projectDir);
    setAgents((prev) => prev.map((a) => (a.agent_id === id ? updated : a)));
  };

  const handleDisconnectAgent = async (id: AgentId) => {
    const updated = await api.disconnectAgent(id);
    setAgents((prev) => prev.map((a) => (a.agent_id === id ? updated : a)));
  };

  const handleConnectWsl = async (distroName: string, homeUnc: string) => {
    try {
      const updated = await api.connectWslAgent(distroName, homeUnc);
      pushToast("success", `WSL (${distroName}) 注入完成：${updated.join(", ") || "无匹配目录"}`);
    } catch (e) {
      pushToast("error", `WSL 注入失败：${String(e)}`);
    }
  };

  const handleImportLegacy = async (localPath: string, agentId: AgentId) => {
    const fileName = localPath.split(/[\\/]/).pop() || "legacy.md";
    const targetUri = `viking://~/resources/inbox/imported-${agentId}-${fileName}`;
    await api.importLegacyMemory(localPath, targetUri);
  };

  const handleTriggerDreaming = async () => {
    if (switchingProfile) return;
    try {
      await api.triggerDreaming();
      setChanges(await api.listPendingChanges());
      setRuns(await api.listDreamingRuns());
      setActiveTab("dreaming");
      pushToast("success", "Dreaming 分析完成，新提案已进入审核台。");
    } catch (e) {
      pushToast("error", `Dreaming 运行失败：${String(e)}`);
    }
  };

  const handleUpdateDecision = async (
    id: string,
    status: ReviewStatus,
    editedAfterContent?: string,
  ) => {
    if (switchingProfile) return;
    await api.updateChangeDecision(id, status, editedAfterContent);
    setChanges(await api.listPendingChanges());
  };

  const handleApplyAccepted = async () => {
    if (switchingProfile) {
      throw new Error("Profile 正在切换，请等待队列刷新后再提交。");
    }
    const report = await api.applyAcceptedChanges();
    setChanges(await api.listPendingChanges());
    setRuns(await api.listDreamingRuns());
    const hasFailed = report.failed_items.length > 0;
    pushToast(
      hasFailed ? "error" : report.conflict_ids.length > 0 ? "info" : "success",
      `已提交 ${report.applied_ids.length} 项，冲突 ${report.conflict_ids.length} 项` +
        (hasFailed ? `，失败 ${report.failed_items.length} 项` : ""),
    );
    return report;
  };

  const handleRollbackRun = async (runId: string) => {
    if (switchingProfile) return;
    try {
      await api.rollbackDreamingRun(runId);
      setRuns(await api.listDreamingRuns());
      pushToast("success", `已通过远端 Snapshot 回滚批次 ${runId}。`);
    } catch (e) {
      pushToast("error", `回滚失败：${String(e)}`);
    }
  };

  const pendingDreamCount = changes.filter((c) => c.status === "pending").length;

  return (
    <div className="h-screen w-screen flex bg-surface-0 text-zinc-100 overflow-hidden">
      <Sidebar
        activeTab={activeTab}
        onSelectTab={setActiveTab}
        serverStatus={serverStatus}
        profiles={profiles}
        onSwitchProfile={handleSwitchProfile}
        onManageProfiles={() => setProfileModalOpen(true)}
        captureEnabled={policy?.global.capture_enabled ?? true}
        onTogglePrivacy={handleTogglePrivacy}
        pendingDreamCount={pendingDreamCount}
        onOpenPalette={() => setPaletteOpen(true)}
      />

      <div className="flex-1 min-w-0 h-full flex flex-col overflow-hidden">
        {/* First-run / Offline Quick-Setup Banner */}
        {serverStatus && !serverStatus.reachable && (
          <div className="px-5 py-2.5 bg-rose-500/10 border-b border-rose-500/25 flex items-center justify-between gap-3 text-xs">
            <div className="flex items-center gap-2 text-rose-200">
              <WifiOff className="w-4 h-4 text-rose-400 shrink-0" />
              <span>
                尚未连接到远端 OpenViking 服务（<code className="font-mono">{serverStatus.url}</code>）。请配置服务地址与 User API Key。
              </span>
            </div>
            <button
              onClick={() => setProfileModalOpen(true)}
              className="px-3 py-1 rounded-md bg-rose-500/20 hover:bg-rose-500/30 border border-rose-400/30 text-rose-100 font-medium shrink-0"
            >
              立即配置远端实例
            </button>
          </div>
        )}

        {/* Privacy Mode Top Reminder */}
        {policy && !policy.global.capture_enabled && (
          <div className="px-5 py-1.5 bg-amber-500/10 border-b border-amber-500/20 flex items-center justify-between text-[11px] text-amber-200">
            <span className="flex items-center gap-1.5">
              <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
              当前处于隐私模式：所有本地 Agent 仅可只读召回，自动捕获与记忆写入已暂停。
            </span>
            <button
              onClick={() => void handleTogglePrivacy()}
              className="underline hover:text-amber-100"
            >
              恢复捕获
            </button>
          </div>
        )}

        <main className="flex-1 min-h-0 overflow-hidden">
          {activeTab === "agents" && (
            <AgentsView
              agents={agents}
              wslDistros={wslDistros}
              onRefresh={async () => {
                setAgents(await api.detectAgents());
                pushToast("info", "已刷新本机 Agent 与桌面端探测状态。");
              }}
              onConnect={handleConnectAgent}
              onDisconnect={handleDisconnectAgent}
              onImportLegacy={handleImportLegacy}
              onConnectWsl={handleConnectWsl}
              onNotify={pushToast}
            />
          )}
          {activeTab === "explorer" && <ExplorerView />}
          {activeTab === "dreaming" && (
            <DreamingWorkbench
              changes={changes}
              runs={runs}
              onTriggerDreaming={handleTriggerDreaming}
              onUpdateDecision={handleUpdateDecision}
              onApplyAccepted={handleApplyAccepted}
              onRollbackRun={handleRollbackRun}
              onPinUri={api.pinMemoryUri}
            />
          )}
          {activeTab === "events" && (
            <EventsView
              agents={agents}
              captureEnabled={policy?.global.capture_enabled ?? true}
            />
          )}
          {activeTab === "policy" && policy && (
            <PolicyView
              policy={policy}
              onSave={async (next) => {
                const saved = await api.savePolicy(next);
                setPolicy(saved);
                pushToast("success", "策略已保存至 ~/.openviking/policy.toml（已热生效）。");
              }}
            />
          )}
        </main>
      </div>

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        onSelectTab={setActiveTab}
        onTriggerDreaming={handleTriggerDreaming}
        onTogglePrivacy={handleTogglePrivacy}
      />

      <ProfileModal
        open={profileModalOpen}
        store={profiles}
        onClose={() => setProfileModalOpen(false)}
        onSaveStore={handleSaveProfiles}
      />

      <ToastStack
        toasts={toasts}
        onDismiss={(id) => setToasts((prev) => prev.filter((t) => t.id !== id))}
      />
    </div>
  );
}
