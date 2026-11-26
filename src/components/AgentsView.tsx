import { useState } from "react";
import { AlertTriangle, CheckCircle2, Plug, RefreshCw, Terminal, Unplug, Upload } from "lucide-react";
import type { AgentId, AgentStatus, WslDistroInfo } from "../types/openviking";

interface AgentsViewProps {
  agents: AgentStatus[];
  wslDistros: WslDistroInfo[];
  onRefresh: () => void;
  onConnect: (id: AgentId, writeRules: boolean) => Promise<void>;
  onDisconnect: (id: AgentId) => Promise<void>;
  onImportLegacy: (localPath: string, agentId: AgentId) => Promise<void>;
}

export function AgentsView({
  agents,
  wslDistros,
  onRefresh,
  onConnect,
  onDisconnect,
  onImportLegacy,
}: AgentsViewProps) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [importingFile, setImportingFile] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [writeAntigravityRules, setWriteAntigravityRules] = useState(true);

  const handleToggle = async (agent: AgentStatus) => {
    setBusyId(agent.id);
    setNotice(null);
    try {
      if (agent.connected) {
        await onDisconnect(agent.agent_id);
        setNotice(`已安全卸载 ${agent.name} 接入配置（原配置已备份）。`);
      } else {
        await onConnect(
          agent.agent_id,
          agent.agent_id === "antigravity" ? writeAntigravityRules : false,
        );
        setNotice(`已成功为 ${agent.name} 注入 ov-bridge 配置。`);
      }
    } finally {
      setBusyId(null);
    }
  };

  const handleImport = async (file: string, agentId: AgentId) => {
    setImportingFile(file);
    setNotice(null);
    try {
      await onImportLegacy(file, agentId);
      setNotice(`已将原生记忆文件 ${file} 冷启动导入至 OpenViking 远端空间。`);
    } catch (e) {
      setNotice(`导入失败：${String(e)}`);
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
            通过内置纯 Rust 单文件 <code className="text-indigo-300 font-mono">ov-bridge</code>{" "}
            统一接入 MCP 与生命周期 Hooks，修改任何配置前自动备份至{" "}
            <code className="font-mono text-zinc-300">~/.openviking/backups/</code>。
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

      {notice && (
        <div className="px-3.5 py-2.5 rounded-lg bg-indigo-500/10 border border-indigo-500/25 text-xs text-indigo-200">
          {notice}
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {agents.map((agent) => (
          <div
            key={agent.id}
            className="rounded-xl bg-surface-1 border border-border-subtle p-4 flex flex-col justify-between gap-4 hover:border-border-strong transition-colors"
          >
            <div className="space-y-2.5">
              <div className="flex items-start justify-between gap-2">
                <div>
                  <div className="flex items-center gap-2">
                    <h2 className="text-sm font-semibold text-zinc-100">{agent.name}</h2>
                    <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-surface-2 text-zinc-400 border border-border-subtle">
                      {agent.integration_mode}
                    </span>
                  </div>
                  <p className="text-[11px] font-mono text-zinc-500 mt-1 truncate">
                    {agent.config_path ?? "未检测到配置路径"}
                  </p>
                </div>
                <span
                  className={`inline-flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-full border ${
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
                <div className="text-[11px] text-zinc-400 font-mono">
                  版本：{agent.version}
                </div>
              )}

              {agent.warning && (
                <div className="flex items-start gap-2 p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/20 text-[11px] text-amber-200/90 leading-relaxed">
                  <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5 text-amber-400" />
                  <span>{agent.warning}</span>
                </div>
              )}

              {agent.agent_id === "antigravity" && !agent.connected && (
                <label className="flex items-center gap-2 text-[11px] text-zinc-400 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={writeAntigravityRules}
                    onChange={(e) => setWriteAntigravityRules(e.target.checked)}
                    className="rounded border-zinc-600 bg-surface-2"
                  />
                  接入时可选补全项目记忆协作规则（适配 MCP-Only 宿主）
                </label>
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
                onClick={() => handleToggle(agent)}
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
                className="px-3 py-2 rounded-lg bg-surface-2 border border-border-subtle text-xs flex items-center justify-between"
              >
                <span className="font-mono text-zinc-200">{d.name}</span>
                <span className="font-mono text-[11px] text-zinc-500 truncate max-w-[260px]">
                  {d.home_unc ?? "UNC 路径待解析"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
