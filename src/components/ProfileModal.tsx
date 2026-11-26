import { useState } from "react";
import { CheckCircle2, Plus, Trash2, Wifi, X, XCircle } from "lucide-react";
import { api } from "../lib/tauri";
import type { OvProfile, ProfileStore, ServerStatus } from "../types/openviking";

interface ProfileModalProps {
  open: boolean;
  store: ProfileStore | null;
  onClose: () => void;
  onSaveStore: (next: ProfileStore) => Promise<void>;
}

export function ProfileModal({ open, store, onClose, onSaveStore }: ProfileModalProps) {
  const [name, setName] = useState("");
  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("http://127.0.0.1:1933");
  const [apiKey, setApiKey] = useState("");
  const [account, setAccount] = useState("");
  const [user, setUser] = useState("");
  const [testResult, setTestResult] = useState<ServerStatus | null>(null);
  const [testing, setTesting] = useState(false);

  if (!open || !store) return null;

  const handleTest = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const res = await api.testConnection(
        url,
        apiKey,
        account || undefined,
        user || undefined,
      );
      setTestResult(res);
    } finally {
      setTesting(false);
    }
  };

  const handleAddOrUpdate = async () => {
    const key = name.trim() || `profile-${Date.now()}`;
    const nextProfile: OvProfile = {
      name: key,
      label: label.trim() || key,
      url: url.trim().replace(/\/+$/, ""),
      api_key: apiKey.trim(),
      account: account.trim() || null,
      user: user.trim() || null,
    };
    const next: ProfileStore = {
      active: key,
      profiles: { ...store.profiles, [key]: nextProfile },
    };
    await onSaveStore(next);
    setName("");
    setLabel("");
    setApiKey("");
    setTestResult(null);
  };

  const handleDelete = async (key: string) => {
    const keys = Object.keys(store.profiles).filter((k) => k !== key);
    if (keys.length === 0) return;
    const profiles = { ...store.profiles };
    delete profiles[key];
    const active = store.active === key ? keys[0] : store.active;
    await onSaveStore({ active, profiles });
  };

  const loadIntoForm = (p: OvProfile) => {
    setName(p.name);
    setLabel(p.label);
    setUrl(p.url);
    setApiKey(p.api_key);
    setAccount(p.account ?? "");
    setUser(p.user ?? "");
    setTestResult(null);
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/65 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-2xl bg-surface-1 border border-border-strong rounded-xl shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-5 py-3.5 border-b border-border-subtle flex items-center justify-between">
          <div>
            <h2 className="text-sm font-semibold text-zinc-100">远端 OpenViking 实例管理</h2>
            <p className="text-[11px] text-zinc-400">
              切换或保存后会自动同步至 <code className="font-mono">~/.openviking/ovcli.conf</code>（请使用 User Key）。
            </p>
          </div>
          <button onClick={onClose} className="p-1 text-zinc-400 hover:text-zinc-200">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="p-5 grid grid-cols-1 md:grid-cols-5 gap-5">
          {/* Existing Profiles */}
          <div className="md:col-span-2 space-y-2">
            <div className="text-[11px] font-semibold text-zinc-400 uppercase">已保存实例</div>
            <div className="space-y-1.5 max-h-72 overflow-y-auto">
              {Object.values(store.profiles).map((p) => (
                <div
                  key={p.name}
                  className={`p-2.5 rounded-lg border text-xs flex items-center justify-between gap-2 ${
                    store.active === p.name
                      ? "bg-indigo-500/10 border-indigo-500/35"
                      : "bg-surface-2 border-border-subtle"
                  }`}
                >
                  <button
                    onClick={() => loadIntoForm(p)}
                    className="text-left min-w-0 flex-1"
                  >
                    <div className="font-medium text-zinc-100 truncate">{p.label}</div>
                    <div className="font-mono text-[10px] text-zinc-500 truncate">{p.url}</div>
                  </button>
                  {Object.keys(store.profiles).length > 1 && (
                    <button
                      onClick={() => void handleDelete(p.name)}
                      className="p-1 text-zinc-500 hover:text-rose-400"
                      title="删除实例"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  )}
                </div>
              ))}
            </div>
          </div>

          {/* Add / Edit Form */}
          <div className="md:col-span-3 space-y-3 text-xs">
            <div className="grid grid-cols-2 gap-2.5">
              <div className="space-y-1">
                <label className="text-zinc-400">Profile ID</label>
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="如 prod-vps"
                  className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-zinc-200"
                />
              </div>
              <div className="space-y-1">
                <label className="text-zinc-400">显示名称</label>
                <input
                  value={label}
                  onChange={(e) => setLabel(e.target.value)}
                  placeholder="如 自建 VPS 远端"
                  className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 text-zinc-200"
                />
              </div>
            </div>

            <div className="space-y-1">
              <label className="text-zinc-400">Server URL</label>
              <input
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="http://127.0.0.1:1933"
                className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-zinc-200"
              />
            </div>

            <div className="space-y-1">
              <label className="text-zinc-400">User API Key（X-API-Key / Bearer）</label>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder="输入 User Key（勿用 Root Key）"
                className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-zinc-200"
              />
            </div>

            <div className="grid grid-cols-2 gap-2.5">
              <div className="space-y-1">
                <label className="text-zinc-400">Account（可选）</label>
                <input
                  value={account}
                  onChange={(e) => setAccount(e.target.value)}
                  placeholder="trusted 模式可选"
                  className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-zinc-200"
                />
              </div>
              <div className="space-y-1">
                <label className="text-zinc-400">User（可选）</label>
                <input
                  value={user}
                  onChange={(e) => setUser(e.target.value)}
                  placeholder="trusted 模式可选"
                  className="w-full bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1.5 font-mono text-zinc-200"
                />
              </div>
            </div>

            {testResult && (
              <div
                className={`p-2.5 rounded-lg border flex items-center gap-2 text-[11px] ${
                  testResult.reachable
                    ? "bg-emerald-500/10 border-emerald-500/25 text-emerald-300"
                    : "bg-rose-500/10 border-rose-500/25 text-rose-300"
                }`}
              >
                {testResult.reachable ? (
                  <CheckCircle2 className="w-4 h-4 shrink-0" />
                ) : (
                  <XCircle className="w-4 h-4 shrink-0" />
                )}
                <span>
                  {testResult.reachable
                    ? `连通正常（延迟 ${testResult.latency_ms} ms）`
                    : `连接失败：${testResult.error ?? "服务不可达"}`}
                </span>
              </div>
            )}

            <div className="flex items-center justify-end gap-2 pt-2">
              <button
                onClick={handleTest}
                disabled={testing || !url.trim()}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-zinc-200"
              >
                <Wifi className="w-3.5 h-3.5" />
                {testing ? "测试中..." : "测试连通性"}
              </button>
              <button
                onClick={handleAddOrUpdate}
                disabled={!url.trim()}
                className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-white font-medium"
              >
                <Plus className="w-3.5 h-3.5" />
                保存并切换
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
