import { useEffect, useState } from "react";
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

  const bootstrap = async () => {
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
    await api.saveProfiles(next);
    setProfiles(next);
    setServerStatus(await api.getServerStatus());
  };

  const handleSwitchProfile = async (name: string) => {
    if (!profiles) return;
    await handleSaveProfiles({ ...profiles, active: name });
  };

  const handleTogglePrivacy = async () => {
    if (!policy) return;
    const updated = await api.togglePrivacyMode(!policy.global.capture_enabled);
    setPolicy(updated);
  };

  const handleConnectAgent = async (id: AgentId, writeRules: boolean) => {
    const updated = await api.connectAgent(id, writeRules);
    setAgents((prev) => prev.map((a) => (a.agent_id === id ? updated : a)));
  };

  const handleDisconnectAgent = async (id: AgentId) => {
    const updated = await api.disconnectAgent(id);
    setAgents((prev) => prev.map((a) => (a.agent_id === id ? updated : a)));
  };

  const handleImportLegacy = async (localPath: string, agentId: AgentId) => {
    const fileName = localPath.split(/[\\/]/).pop() || "legacy.md";
    const targetUri = `viking://~/resources/inbox/imported-${agentId}-${fileName}`;
    await api.importLegacyMemory(localPath, targetUri);
  };

  const handleTriggerDreaming = async () => {
    await api.triggerDreaming();
    setChanges(await api.listPendingChanges());
    setRuns(await api.listDreamingRuns());
    setActiveTab("dreaming");
  };

  const handleUpdateDecision = async (
    id: string,
    status: ReviewStatus,
    editedAfterContent?: string,
  ) => {
    await api.updateChangeDecision(id, status, editedAfterContent);
    setChanges(await api.listPendingChanges());
  };

  const handleApplyAccepted = async () => {
    const report = await api.applyAcceptedChanges();
    setChanges(await api.listPendingChanges());
    setRuns(await api.listDreamingRuns());
    return report;
  };

  const handleRollbackRun = async (runId: string) => {
    await api.rollbackDreamingRun(runId);
    setRuns(await api.listDreamingRuns());
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

      <main className="flex-1 min-w-0 h-full overflow-hidden">
        {activeTab === "agents" && (
          <AgentsView
            agents={agents}
            wslDistros={wslDistros}
            onRefresh={async () => setAgents(await api.detectAgents())}
            onConnect={handleConnectAgent}
            onDisconnect={handleDisconnectAgent}
            onImportLegacy={handleImportLegacy}
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
        {activeTab === "events" && <EventsView />}
        {activeTab === "policy" && policy && (
          <PolicyView
            policy={policy}
            onSave={async (next) => {
              const saved = await api.savePolicy(next);
              setPolicy(saved);
            }}
          />
        )}
      </main>

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
    </div>
  );
}
