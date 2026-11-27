import { useEffect, useRef, useState } from "react";
import {
  CornerLeftUp,
  FilePlus2,
  FileText,
  Folder,
  Inbox,
  Info,
  RefreshCw,
  Save,
  Search,
  Trash2,
} from "lucide-react";
import { api } from "../lib/tauri";
import type { ContextLayer, LayeredContent } from "../types/openviking";

const ROOT_SCOPES = [
  { label: "用户共享记忆", uri: "viking://~/memories/" },
  { label: "项目知识库", uri: "viking://resources/projects/" },
  { label: "Inbox 待分拣区", uri: "viking://~/resources/inbox/" },
  { label: "Agent 私有空间", uri: "viking://agent/" },
];

function normalizeDirUri(uri: string): string {
  return uri.endsWith("/") ? uri : `${uri}/`;
}

function parentDirUri(uri: string): string {
  const clean = uri.replace(/\/+$/, "");
  const idx = clean.lastIndexOf("/");
  if (idx <= "viking://".length) return normalizeDirUri(clean);
  return `${clean.slice(0, idx + 1)}`;
}

function relativeDisplayPath(uri: string, baseUri: string): string {
  const normBase = normalizeDirUri(baseUri);
  if (uri.startsWith(normBase)) return uri.slice(normBase.length);
  return uri.replace(/^viking:\/\//, "");
}

export function ExplorerView() {
  const [rootScopeUri, setRootScopeUri] = useState(ROOT_SCOPES[0].uri);
  const [scopeUri, setScopeUri] = useState(ROOT_SCOPES[0].uri);
  const [dirStack, setDirStack] = useState<string[]>([]);
  const [entries, setEntries] = useState<
    Array<{ name: string; uri: string; isDir?: boolean; size?: number }>
  >([]);
  const [submittedSearch, setSubmittedSearch] = useState<string | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [selectedUri, setSelectedUri] = useState<string | null>(null);
  const [loadedUri, setLoadedUri] = useState<string | null>(null);
  const [loadingLayer, setLoadingLayer] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [layers, setLayers] = useState<LayeredContent | null>(null);
  const [activeLayer, setActiveLayer] = useState<ContextLayer>("l2");
  const [draftL2, setDraftL2] = useState("");
  const [saving, setSaving] = useState(false);
  const [savedNotice, setSavedNotice] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [newFileName, setNewFileName] = useState("");
  const [showNewInput, setShowNewInput] = useState(false);
  const readReqIdRef = useRef(0);
  const listReqIdRef = useRef(0);

  const canGoUp = dirStack.length > 0;
  const isDirtyL2 = Boolean(layers && loadedUri === selectedUri && draftL2 !== layers.l2);

  const loadList = async (uri: string) => {
    const reqId = ++listReqIdRef.current;
    setSubmittedSearch(null);
    setListError(null);
    try {
      const list = await api.fsLs(uri);
      if (listReqIdRef.current !== reqId) return;
      setEntries(list);
      const firstFile = list.find((i) => !i.isDir);
      if (firstFile && !selectedUri) setSelectedUri(firstFile.uri);
    } catch (err) {
      if (listReqIdRef.current !== reqId) return;
      setListError(`目录加载失败：${String(err)}`);
    }
  };

  useEffect(() => {
    void loadList(scopeUri);
  }, [scopeUri]);

  useEffect(() => {
    const reqId = ++readReqIdRef.current;
    if (!selectedUri) {
      setLoadedUri(null);
      setLayers(null);
      setDraftL2("");
      setLoadError(null);
      setSaveError(null);
      setLoadingLayer(false);
      return;
    }
    const targetUri = selectedUri;
    setLoadedUri(null);
    setLoadError(null);
    setSaveError(null);
    setSavedNotice(null);
    setLoadingLayer(true);
    api
      .readUriLayers(targetUri)
      .then((data) => {
        if (readReqIdRef.current !== reqId) return;
        setLayers(data);
        setDraftL2(data.l2);
        setLoadedUri(targetUri);
        setLoadError(null);
      })
      .catch((err) => {
        if (readReqIdRef.current !== reqId) return;
        setLayers(null);
        setDraftL2("");
        setLoadedUri(null);
        setLoadError(`读取失败（已锁定保存防覆盖）：${String(err)}`);
      })
      .finally(() => {
        if (readReqIdRef.current === reqId) setLoadingLayer(false);
      });
  }, [selectedUri, refreshNonce]);

  const canSaveL2 =
    Boolean(selectedUri) && loadedUri === selectedUri && !loadingLayer && !loadError && !saving;

  const handleSave = async () => {
    if (!selectedUri || loadedUri !== selectedUri || loadingLayer || loadError) return;
    const targetUri = selectedUri;
    setSaving(true);
    setSaveError(null);
    try {
      await api.writeUriContent(targetUri, draftL2);
      if (loadedUri === targetUri) {
        setLayers((prev) => (prev ? { ...prev, l2: draftL2 } : prev));
        setSavedNotice("已写回远端并触发语义索引刷新");
      }
    } catch (err) {
      setSaveError(`保存失败（可重试）：${String(err)}`);
    } finally {
      setSaving(false);
    }
  };

  const runSearch = async (query: string, targetScope: string) => {
    const reqId = ++listReqIdRef.current;
    setSubmittedSearch(query);
    setListError(null);
    try {
      const found = await api.searchFind(query, targetScope);
      if (listReqIdRef.current !== reqId) return;
      setEntries(found);
    } catch (err) {
      if (listReqIdRef.current !== reqId) return;
      setListError(`搜索失败：${String(err)}`);
    }
  };

  const handleSearch = async () => {
    const clean = searchQuery.trim();
    if (!clean) {
      await loadList(scopeUri);
      return;
    }
    await runSearch(clean, scopeUri);
  };

  const handleRefreshInPlace = async () => {
    setRefreshing(true);
    try {
      if (submittedSearch !== null) {
        await runSearch(submittedSearch, scopeUri);
      } else {
        await loadList(scopeUri);
      }
      if (selectedUri) {
        if (isDirtyL2) {
          setSavedNotice("已刷新列表；当前文件有未保存的 L2 修改，已保留草稿。");
        } else {
          setRefreshNonce((n) => n + 1);
        }
      }
    } finally {
      setRefreshing(false);
    }
  };

  const handleGoUp = () => {
    if (dirStack.length === 0) return;
    const prev = dirStack[dirStack.length - 1];
    setDirStack((stack) => stack.slice(0, -1));
    setSearchQuery("");
    setSubmittedSearch(null);
    setScopeUri(prev);
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
                  setSubmittedSearch(null);
                  setRootScopeUri(s.uri);
                  setDirStack([]);
                  if (scopeUri === s.uri) {
                    void loadList(s.uri);
                  } else {
                    setScopeUri(s.uri);
                  }
                }}
                className={`inline-flex items-center gap-1 px-2 py-1 rounded-md text-[11px] border transition-colors ${
                  rootScopeUri === s.uri
                    ? "bg-indigo-500/15 text-indigo-300 border-indigo-500/30"
                    : "bg-surface-2 text-zinc-400 border-border-subtle hover:text-zinc-200"
                }`}
              >
                {s.uri.includes("inbox") && <Inbox className="w-3 h-3" />}
                {s.label}
              </button>
            ))}
          </div>

          <div className="flex items-center gap-1.5 pt-0.5">
            <button
              onClick={handleGoUp}
              disabled={!canGoUp}
              className="p-1 rounded-md bg-surface-2 hover:bg-surface-3 border border-border-subtle text-zinc-300 disabled:opacity-35 disabled:hover:bg-surface-2 shrink-0"
              title="返回上一级目录"
            >
              <CornerLeftUp className="w-3.5 h-3.5" />
            </button>
            <div
              className="flex-1 min-w-0 px-2 py-1 rounded-md bg-surface-0 border border-border-subtle font-mono text-[10px] text-zinc-400 truncate"
              title={scopeUri}
            >
              {scopeUri}
            </div>
            <button
              onClick={() => void handleRefreshInPlace()}
              disabled={refreshing}
              className="p-1 rounded-md bg-surface-2 hover:bg-surface-3 border border-border-subtle text-zinc-300 disabled:opacity-40 shrink-0"
              title="就地刷新当前目录与预览"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? "animate-spin" : ""}`} />
            </button>
          </div>
          {listError && (
            <div className="px-2 py-1.5 rounded-md bg-rose-500/10 border border-rose-500/25 text-[10px] text-rose-300">
              {listError}
            </div>
          )}
        </div>

        <div className="flex-1 overflow-y-auto p-2 space-y-1">
          {entries.map((item) => {
            const relPath = relativeDisplayPath(item.uri, scopeUri);
            const showSubpath = submittedSearch !== null || relPath !== item.name;
            return (
              <div
                key={item.uri}
                className={`group flex items-center justify-between px-2.5 py-2 rounded-lg text-xs transition-colors ${
                  selectedUri === item.uri
                    ? "bg-indigo-500/15 text-indigo-200 border border-indigo-500/25"
                    : "text-zinc-300 hover:bg-surface-2 border border-transparent"
                }`}
              >
                <button
                  onClick={() => {
                    if (item.isDir) {
                      setDirStack((stack) => [...stack, scopeUri]);
                      setSearchQuery("");
                      setSubmittedSearch(null);
                      setScopeUri(normalizeDirUri(item.uri));
                    } else {
                      setSelectedUri(item.uri);
                    }
                  }}
                  title={item.uri}
                  className="flex items-center gap-2 min-w-0 flex-1 text-left"
                >
                  {item.isDir ? (
                    <Folder className="w-3.5 h-3.5 text-amber-400 shrink-0" />
                  ) : (
                    <FileText className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-mono text-[11px]">{item.name}</div>
                    {showSubpath && (
                      <div className="truncate font-mono text-[10px] text-zinc-500">
                        {relPath}
                      </div>
                    )}
                  </div>
                </button>
                {!item.isDir && (
                  <button
                    onClick={() => void handleDelete(item.uri)}
                    className="opacity-0 group-hover:opacity-100 p-0.5 text-zinc-500 hover:text-rose-400 transition-opacity shrink-0"
                    title="删除条目"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                )}
              </div>
            );
          })}
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
                disabled={!canSaveL2}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-xs font-medium text-white disabled:opacity-50"
              >
                <Save className="w-3.5 h-3.5" />
                {saving ? "保存中..." : loadingLayer ? "加载中..." : "保存 L2"}
              </button>
            )}
          </div>
        </div>

        <div className="flex-1 p-5 overflow-y-auto">
          {loadError && (
            <div className="mb-3 px-3 py-2 rounded-lg bg-rose-500/10 border border-rose-500/25 text-xs text-rose-300">
              {loadError}
            </div>
          )}
          {saveError && (
            <div className="mb-3 px-3 py-2 rounded-lg bg-amber-500/10 border border-amber-500/25 text-xs text-amber-200">
              {saveError}
            </div>
          )}
          {savedNotice && (
            <div className="mb-3 px-3 py-2 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-xs text-emerald-300">
              {savedNotice}
            </div>
          )}
          {activeLayer === "l2" ? (
            <textarea
              value={draftL2}
              disabled={!selectedUri || loadedUri !== selectedUri || loadingLayer || Boolean(loadError)}
              onChange={(e) => setDraftL2(e.target.value)}
              placeholder={loadingLayer ? "正在加载远端 L2 原文..." : "请选择左侧文件"}
              className="w-full h-full min-h-[420px] bg-surface-1 border border-border-subtle rounded-xl p-4 font-mono text-xs text-zinc-200 leading-relaxed focus:outline-none focus:border-indigo-500/40 resize-none disabled:opacity-60"
            />
          ) : (
            <div className="space-y-2.5">
              {selectedUri && !selectedUri.endsWith("/") && (
                <div className="flex items-start gap-2 px-3 py-2 rounded-lg bg-sky-500/10 border border-sky-500/20 text-[11px] text-sky-200/90">
                  <Info className="w-3.5 h-3.5 shrink-0 mt-0.5 text-sky-400" />
                  <span>
                    OpenViking 的 {activeLayer.toUpperCase()} 摘要按目录生成，当前展示所属目录{" "}
                    <code className="font-mono text-sky-100">{parentDirUri(selectedUri)}</code>{" "}
                    的摘要；查看单文件原文请切换至 <strong className="font-mono">L2</strong>。
                  </span>
                </div>
              )}
              <pre className="w-full bg-surface-1 border border-border-subtle rounded-xl p-4 font-mono text-xs text-zinc-300 whitespace-pre-wrap leading-relaxed">
                {activeLayer === "l0" ? layers?.l0 || "暂无 L0 摘要" : layers?.l1 || "暂无 L1 概览"}
              </pre>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
