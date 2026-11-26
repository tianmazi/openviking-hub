import { useEffect, useState } from "react";
import { api } from "../lib/tauri";
import {
  AlertTriangle,
  CheckCircle2,
  Copy,
  Monitor,
  Plug,
  RefreshCw,
  Terminal,
  Unplug,
  Upload,
} from "lucide-react";
import type { AgentId, AgentStatus, CaptureHealth, WslDistroInfo } from "../types/openviking";

interface AgentsViewProps {
  agents: AgentStatus[];
  wslDistros: WslDistroInfo[];
  onRefresh: () => void;
  onConnect: (id: AgentId, writeRules: boolean, projectDir?: string) => Promise<void>;
  onDisconnect: (id: AgentId) => Promise<void>;
  onImportLegacy: (localPath: string, agentId: AgentId) => Promise<void>;
  onConnectWsl: (distroName: string, homeUnc: string) => Promise<void>;
  onNotify: (kind: "success" | "error" | "info", message: string) => void;
}

export function AgentsView({
  agents,
  wslDistros,
  onRefresh,
  onConnect,
  onDisconnect,
  onImportLegacy,
  onConnectWsl,
  onNotify,
}: AgentsViewProps) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [captureHealth, setCaptureHealth] = useState<CaptureHealth | null>(null);
  useEffect(() => {
    let active = true;
    api.getCaptureHealth().then(value => { if (active) setCaptureHealth(value); })
      .catch(() => { if (active) setCaptureHealth(null); });
    return () => { active = false; };
  }, [agents]);
  const [importingFile, setImportingFile] = useState<string | null>(null);
  const [writeAntigravityRules, setWriteAntigravityRules] = useState(false);
  const [antigravityProjectDir, setAntigravityProjectDir] = useState("");

  const handleToggle = async (agent: AgentStatus) => {
    if (
      !agent.connected &&
      agent.agent_id === "antigravity" &&
      writeAntigravityRules &&
      !antigravityProjectDir.trim()
    ) {
      onNotify("error", "勾选项目规则时请先填写项目目录路径。");
      return;
    }
    setBusyId(agent.id);
    try {
      if (agent.connected) {
        await onDisconnect(agent.agent_id);
        onNotify("info", `已卸载 ${agent.name} 接入配置（原配置已备份至 ~/.openviking/backups/）。`);
      } else {
        await onConnect(
          agent.agent_id,
          agent.agent_id === "antigravity" ? writeAntigravityRules : false,
          agent.agent_id === "antigravity" && writeAntigravityRules
            ? antigravityProjectDir.trim()
            : undefined,
        );
        onNotify("success", `已成功为 ${agent.name} 注入 ov-bridge 配置。`);
      }
    } catch (e) {
      onNotify("error", `${agent.name} 操作失败：${String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const handleCopyPath = async (path: string) => {
    try {
      await navigator.clipboard.writeText(path);
      onNotify("info", `已复制配置路径：${path}`);
    } catch {
      onNotify("info", `配置路径：${path}`);
    }
  };

  const handleImport = async (file: string, agentId: AgentId) => {
    setImportingFile(file);
    try {
      await onImportLegacy(file, agentId);
      onNotify("success", `已将原生记忆文件 ${file} 导入至远端 Inbox。`);
    } catch (e) {
      onNotify("error", `导入失败：${String(e)}`);
    } finally {
      setImportingFile(null);
    }
  };

  return (
    <div className="h-full overflow-y-auto p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-zinc-100">本地 Agent 自动探测与接入</h1>
          <p className="text-xs text-zinc-400 mt-1">
            同时支持{" "}
            <strong className="text-zinc-200 font-medium">Desktop App 桌面端与 CLI</strong>
            （Codex / Antigravity / Claude Code / pi），通过纯 Rust{" "}
            <code className="text-indigo-300 font-mono">ov-bridge</code> 统一接入并自动备份。
          </p>
        </div>
        <button
          onClick={onRefresh}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-xs text-zinc-200"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          重新探测
        </button>
      </div>

      <section className="rounded-lg border border-border p-3 text-sm">
        <h3 className="font-medium mb-2">当前 Profile · 记忆同步</h3>
        {captureHealth ? (
          <>
            <p>待发送 {captureHealth.pending} · 已发送待归档 {captureHealth.sent} · 待核对会话 {captureHealth.uncertain_sessions}</p>
            <p className="text-xs text-muted-foreground mt-1">最近记录的提取任务：{captureHealth.extraction_tasks.length} · 完成 {captureHealth.extraction_tasks.filter(([, status]) => status === "completed").length}</p>
            {captureHealth.last_error && <p className="text-xs text-red-500 mt-1">{captureHealth.last_error}</p>}
          </>
        ) : <p className="text-muted-foreground">同步状态暂不可用</p>}
      </section>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {agents.map((agent) => (
          <div
            key={agent.id}
            className="rounded-xl bg-surface-1 border border-border-subtle p-4 flex flex-col justify-between gap-4 hover:border-border-strong transition-colors"
          >
            <div className="space-y-2.5">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <h2 className="text-sm font-semibold text-zinc-100">{agent.name}</h2>
                    {agent.detected_surfaces.map((surface) => (
                      <span
                        key={surface}
                        className="inline-flex items-center gap-1 text-[10px] font-mono px-1.5 py-0.5 rounded bg-indigo-500/15 text-indigo-300 border border-indigo-500/25"
                      >
                        <Monitor className="w-2.5 h-2.5" />
                        {surface}
                      </span>
                    ))}
                  </div>
                  <div className="text-[10px] font-mono text-zinc-500 mt-1">
                    {agent.integration_mode}
                  </div>
                  {agent.config_path && (
                    <div className="flex items-center gap-1.5 mt-1">
                      <span className="text-[11px] font-mono text-zinc-400 truncate">
                        {agent.config_path}
                      </span>
                      <button
                        onClick={() => void handleCopyPath(agent.config_path!)}
                        className="p-0.5 text-zinc-500 hover:text-zinc-200 shrink-0"
                        title="复制配置文件路径"
                      >
                        <Copy className="w-3 h-3" />
                      </button>
                    </div>
                  )}
                </div>
                <span
                  className={`inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full border shrink-0 ${
                    agent.connected
                      ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/20"
                      : agent.installed
                        ? "bg-zinc-800 text-zinc-400 border-zinc-700"
                        : "bg-zinc-900 text-zinc-500 border-zinc-800"
                  }`}
                >
                  {agent.connected && <CheckCircle2 className="w-3 h-3" />}
                  {agent.connected ? "已接入" : agent.installed ? "未接入" : "未安装"}
                </span>
              </div>

              {agent.version && (
                <div className="text-[11px] text-zinc-400 font-mono">版本：{agent.version}</div>
              )}

              {agent.warning && (
                <div className="flex items-start gap-2 p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/20 text-[11px] text-amber-200/90 leading-relaxed">
                  <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5 text-amber-400" />
                  <span>{agent.warning}</span>
                </div>
              )}

              {agent.agent_id === "antigravity" && !agent.connected && (
                <div className="space-y-1.5">
                  <label className="flex items-center gap-2 text-[11px] text-zinc-400 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={writeAntigravityRules}
                      onChange={(e) => setWriteAntigravityRules(e.target.checked)}
                      className="rounded border-zinc-600 bg-surface-2"
                    />
                    同时写入项目记忆协作规则（与全局 hooks.json 互补）
                  </label>
                  {writeAntigravityRules && (
                    <input
                      value={antigravityProjectDir}
                      onChange={(e) => setAntigravityProjectDir(e.target.value)}
                      placeholder="项目根目录绝对路径，例如 E:\git_work\my-project"
                      className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-[11px] text-zinc-200 focus:outline-none focus:border-indigo-500/40"
                    />
                  )}
                </div>
              )}

              {agent.legacy_memory_files.length > 0 && (
                <div className="space-y-1.5 pt-1">
                  <div className="text-[11px] text-zinc-400">原生历史记忆（冷启动导入）：</div>
                  {agent.legacy_memory_files.map((file) => (
                    <div
                      key={file}
                      className="flex items-center justify-between gap-2 px-2.5 py-1.5 rounded-lg bg-surface-2 border border-border-subtle text-[11px]"
                    >
                      <span className="font-mono text-indigo-300 truncate">{file}</span>
                      <button
                        disabled={importingFile === file}
                        onClick={() => void handleImport(file, agent.agent_id)}
                        className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-indigo-500/15 hover:bg-indigo-500/25 text-indigo-200 shrink-0"
                      >
                        <Upload className="w-3 h-3" />
                        {importingFile === file ? "导入中..." : "导入远端"}
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-border-subtle">
              <button
                disabled={busyId === agent.id}
                onClick={() => void handleToggle(agent)}
                className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors ${
                  agent.connected
                    ? "bg-surface-2 hover:bg-rose-500/20 hover:text-rose-300 text-zinc-300 border border-border-subtle"
                    : "bg-indigo-600 hover:bg-indigo-500 text-white"
                }`}
              >
                {agent.connected ? (
                  <>
                    <Unplug className="w-3.5 h-3.5" />
                    {busyId === agent.id ? "回滚中..." : "卸载并回滚"}
                  </>
                ) : (
                  <>
                    <Plug className="w-3.5 h-3.5" />
                    {busyId === agent.id ? "注入中..." : "一键接入"}
                  </>
                )}
              </button>
            </div>
          </div>
        ))}
      </div>

      {wslDistros.length > 0 && (
        <div className="rounded-xl bg-surface-1 border border-border-subtle p-4 space-y-2">
          <div className="flex items-center gap-2 text-xs font-semibold text-zinc-200">
            <Terminal className="w-4 h-4 text-indigo-400" />
            检测到 Windows WSL 发行版
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
            {wslDistros.map((d) => (
              <div
                key={d.name}
                className="px-3 py-2 rounded-lg bg-surface-2 border border-border-subtle text-xs flex items-center justify-between gap-2"
              >
                <div className="min-w-0">
                  <div className="font-mono text-zinc-200">{d.name}</div>
                  <div className="font-mono text-[10px] text-zinc-500 truncate">
                    {d.home_unc ?? "UNC 路径待解析"}
                  </div>
                </div>
                {d.home_unc && (
                  <button
                    onClick={() => void onConnectWsl(d.name, d.home_unc!)}
                    className="px-2.5 py-1 rounded-md bg-indigo-600 hover:bg-indigo-500 text-[11px] text-white shrink-0"
                  >
                    一键注入 WSL
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
