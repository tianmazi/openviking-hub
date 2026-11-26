import { useEffect, useState } from "react";
import { FilePlus2, FileText, Folder, Inbox, Save, Search, Trash2 } from "lucide-react";
import { api } from "../lib/tauri";
import type { ContextLayer, LayeredContent } from "../types/openviking";

const ROOT_SCOPES = [
  { label: "用户共享记忆", uri: "viking://user/memories/" },
  { label: "项目知识库", uri: "viking://resources/projects/" },
  { label: "Inbox 待分拣区", uri: "viking://~/resources/inbox/" },
];

export function ExplorerView() {
  const [scopeUri, setScopeUri] = useState(ROOT_SCOPES[0].uri);
  const [entries, setEntries] = useState<
    Array<{ name: string; uri: string; isDir?: boolean; size?: number }>
  >([]);
  const [selectedUri, setSelectedUri] = useState<string | null>(null);
  const [layers, setLayers] = useState<LayeredContent | null>(null);
  const [activeLayer, setActiveLayer] = useState<ContextLayer>("l2");
  const [draftL2, setDraftL2] = useState("");
  const [saving, setSaving] = useState(false);
  const [savedNotice, setSavedNotice] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [newFileName, setNewFileName] = useState("");
  const [showNewInput, setShowNewInput] = useState(false);

  const loadList = async (uri: string) => {
    const list = await api.fsLs(uri);
    setEntries(list);
    const firstFile = list.find((i) => !i.isDir);
    if (firstFile && !selectedUri) setSelectedUri(firstFile.uri);
  };

  useEffect(() => {
    void loadList(scopeUri);
  }, [scopeUri]);

  useEffect(() => {
    if (!selectedUri) return;
    api.readUriLayers(selectedUri).then((data) => {
      setLayers(data);
      setDraftL2(data.l2);
      setSavedNotice(null);
    });
  }, [selectedUri]);

  const handleSave = async () => {
    if (!selectedUri) return;
    setSaving(true);
    try {
      await api.writeUriContent(selectedUri, draftL2);
      setLayers((prev) => (prev ? { ...prev, l2: draftL2 } : prev));
      setSavedNotice("已写回远端并触发语义索引刷新");
    } finally {
      setSaving(false);
    }
  };

  const handleSearch = async () => {
    if (!searchQuery.trim()) {
      void loadList(scopeUri);
      return;
    }
    const found = await api.searchFind(searchQuery.trim(), scopeUri);
    setEntries(found);
  };

  const handleCreateNote = async () => {
    const clean = newFileName.trim();
    if (!clean) return;
    const file = clean.endsWith(".md") ? clean : `${clean}.md`;
    const target = `${scopeUri.replace(/\/+$/, "")}/${file}`;
    await api.writeUriContent(target, `# ${file.replace(/\.md$/, "")}\n\n`);
    setNewFileName("");
    setShowNewInput(false);
    await loadList(scopeUri);
    setSelectedUri(target);
  };

  const handleDelete = async (uri: string) => {
    await api.fsDelete(uri, false);
    if (selectedUri === uri) {
      setSelectedUri(null);
      setLayers(null);
      setDraftL2("");
    }
    await loadList(scopeUri);
  };

  return (
    <div className="h-full flex">
      {/* Middle File List */}
      <div className="w-80 shrink-0 border-r border-border-subtle bg-surface-1/60 flex flex-col">
        <div className="p-3 border-b border-border-subtle space-y-2.5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-zinc-200">viking:// 虚拟文件系统</span>
            <button
              onClick={() => setShowNewInput((v) => !v)}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-surface-2 hover:bg-surface-3 border border-border-subtle text-[11px] text-zinc-300"
            >
              <FilePlus2 className="w-3 h-3 text-indigo-400" />
              新建
            </button>
          </div>

          <div className="flex gap-1.5">
            <input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void handleSearch()}
              placeholder="语义搜索当前作用域..."
              className="flex-1 bg-surface-2 border border-border-subtle rounded-lg px-2.5 py-1 text-[11px] text-zinc-200 focus:outline-none focus:border-indigo-500/40"
            />
            <button
              onClick={() => void handleSearch()}
              className="px-2 py-1 rounded-lg bg-surface-2 hover:bg-surface-3 border border-border-subtle text-zinc-300"
              title="执行语义搜索"
            >
              <Search className="w-3.5 h-3.5" />
            </button>
          </div>

          {showNewInput && (
            <div className="flex gap-1.5">
              <input
                autoFocus
                value={newFileName}
                onChange={(e) => setNewFileName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void handleCreateNote()}
                placeholder="如 preferences/rust-style.md"
                className="flex-1 bg-surface-0 border border-indigo-500/40 rounded-lg px-2 py-1 font-mono text-[11px] text-zinc-200"
              />
              <button
                onClick={() => void handleCreateNote()}
                className="px-2.5 py-1 rounded-lg bg-indigo-600 text-[11px] text-white"
              >
                创建
              </button>
            </div>
          )}

          <div className="flex flex-wrap gap-1">
            {ROOT_SCOPES.map((s) => (
              <button
                key={s.uri}
                onClick={() => {
                  setSearchQuery("");
                  setScopeUri(s.uri);
                }}
                className={`inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] border transition-colors ${
                  scopeUri === s.uri
                    ? "bg-indigo-500/15 text-indigo-300 border-indigo-500/30"
                    : "bg-surface-2 text-zinc-400 border-border-subtle hover:text-zinc-200"
                }`}
              >
                {s.uri.includes("inbox") && <Inbox className="w-3 h-3" />}
                {s.label}
              </button>
            ))}
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-2 space-y-1">
          {entries.map((item) => (
            <div
              key={item.uri}
              className={`group flex items-center justify-between px-2.5 py-2 rounded-lg text-xs transition-colors ${
                selectedUri === item.uri
                  ? "bg-indigo-500/15 text-indigo-200 border border-indigo-500/25"
                  : "text-zinc-300 hover:bg-surface-2 border border-transparent"
              }`}
            >
              <button
                onClick={() => (item.isDir ? setScopeUri(item.uri) : setSelectedUri(item.uri))}
                className="flex items-center gap-2 truncate flex-1 text-left"
              >
                {item.isDir ? (
                  <Folder className="w-3.5 h-3.5 text-amber-400 shrink-0" />
                ) : (
                  <FileText className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
                )}
                <span className="truncate font-mono text-[11px]">{item.name}</span>
              </button>
              {!item.isDir && (
                <button
                  onClick={() => void handleDelete(item.uri)}
                  className="opacity-0 group-hover:opacity-100 p-0.5 text-zinc-500 hover:text-rose-400 transition-opacity"
                  title="删除条目"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Right L0 / L1 / L2 Inspector & Editor */}
      <div className="flex-1 flex flex-col bg-surface-0">
        <div className="px-5 py-3 border-b border-border-subtle flex items-center justify-between gap-4">
          <div className="font-mono text-xs text-zinc-300 truncate">
            {selectedUri ?? "请选择左侧条目"}
          </div>
          <div className="flex items-center gap-2">
            <div className="inline-flex rounded-lg bg-surface-1 p-0.5 border border-border-subtle">
              {(["l0", "l1", "l2"] as ContextLayer[]).map((layer) => (
                <button
                  key={layer}
                  onClick={() => setActiveLayer(layer)}
                  className={`px-2.5 py-1 rounded-md text-[11px] font-mono uppercase transition-colors ${
                    activeLayer === layer
                      ? "bg-indigo-600 text-white"
                      : "text-zinc-400 hover:text-zinc-200"
                  }`}
                >
                  {layer}
                </button>
              ))}
            </div>
            {activeLayer === "l2" && (
              <button
                onClick={handleSave}
                disabled={saving || !selectedUri}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-xs font-medium text-white disabled:opacity-50"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? "保存中..." : "保存 L2"}
              </button>
            )}
          </div>
        </div>

        <div className="flex-1 p-5 overflow-y-auto">
          {savedNotice && (
            <div className="mb-3 px-3 py-2 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-xs text-emerald-300">
              {savedNotice}
            </div>
          )}
          {activeLayer === "l2" ? (
            <textarea
              value={draftL2}
              onChange={(e) => setDraftL2(e.target.value)}
              className="w-full h-full min-h-[420px] bg-surface-1 border border-border-subtle rounded-xl p-4 font-mono text-xs text-zinc-200 leading-relaxed focus:outline-none focus:border-indigo-500/40 resize-none"
            />
          ) : (
            <pre className="w-full bg-surface-1 border border-border-subtle rounded-xl p-4 font-mono text-xs text-zinc-300 whitespace-pre-wrap leading-relaxed">
              {activeLayer === "l0" ? layers?.l0 || "暂无 L0 摘要" : layers?.l1 || "暂无 L1 概览"}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
