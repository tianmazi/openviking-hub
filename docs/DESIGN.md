# OpenViking Desktop 架构与设计方案

> 基于 Tauri v2 + Rust 3-Crate Workspace 的跨平台 OpenViking 本地控制平面、统一 MCP/Hooks 桥接引擎与 Dreaming 记忆治理控制台。

## 1. 设计目标与差异化定位

相比官方 Beta 版 `openviking-helper`，本项目围绕以下三个核心痛点进行架构升级：

1. **现代克制的桌面体验**：采用 React + TypeScript + Tailwind CSS 构建 Linear / Raycast 风格的深色三栏主控制台 + 系统托盘优先（Tray-First）交互，内置全局 `Cmd+K` 快捷搜索与一键隐私模式。
2. **可扩展的跨平台 Agent 适配与零依赖桥接**：
   - 支持 **Claude Code、Codex、pi、Antigravity（反重力）** 四大本地 Agent，并兼容 **macOS / Linux / Windows（含 Windows WSL 发行版自动探测）**。
   - 使用独立编译的纯 Rust 单文件 Sidecar `ov-bridge`（同时承担 `stdio MCP Server` 与生命周期 `Hook` 执行），彻底消除用户本机 Node.js / `npx.cmd` / Python / Bash 兼容性问题。
3. **内置记忆治理策略引擎（无写入前脱敏）**：
   - **作用域路由（Scope Router）**：`RESOURCE_POLICY.md` > 手动映射 > Git Remote 自动识别 > `viking://~/resources/inbox/` 兜底，并对 `CLAUDE.md`、`AGENTS.md` 等人工强约束规则文件启用只读保护。
   - **分层注入与 Token 预算（Tiered Loading & Budgeting）**：严格限制会话启动注入上限（默认 200 行 / 24KB，Claude Code 覆写为 9500 bytes），支持 L0/L1/L2 渐进式召回与六类记忆分类配额。
   - **Dreaming 后台整理与 PR-Style 审核台**：采用 **Stage-then-Commit** 模型——只读分析产出待审提案并记录远端内容 SHA-256 `base_hash`，人工 Diff 审阅/直接编辑终稿后，在 Commit 前自动调用 OpenViking Snapshots API 创建恢复点，支持乐观锁防并发漂移与一键回滚。

## 2. 工程架构（3-Crate Cargo Workspace）

```text
openviking-desktop/
├── crates/
│   ├── ov-core/       # 共享核心库：OpenViking REST 客户端、Policy 引擎、跨平台路径、4 Agent 适配器、Dreaming 引擎与 SQLite
│   └── ov-bridge/     # 纯 Rust 单文件 Sidecar：`ov-bridge mcp` (stdio JSON-RPC 2.0) + `ov-bridge hook <event>`
├── src-tauri/         # Tauri v2 桌面外壳：IPC Commands、系统托盘、Dreaming 定时调度器
└── src/               # React + TypeScript + Tailwind CSS 三栏控制台与 Dreaming PR 审核台
```

## 3. OpenViking REST API 对齐规范

所有接口严格对齐 [OpenViking 官方 HTTP API 规范](https://docs.openviking.ai/en/api/01-overview)：

- **鉴权与配置**：托管 `~/.openviking/ovcli.conf`（`url`、`api_key`、`account`、`user`、`timeout`、`output`），优先发送 `Authorization: Bearer <key>` 并兼容 `X-API-Key: <key>`；统一解析 `{"status":"ok","result":...}` 与 `{"status":"error","error":{"code","message"}}` 响应信封。
- **健康与观测**：`GET /health`、`GET /ready`、`GET /api/v1/system/status`、`GET /api/v1/observer/system`。
- **虚拟文件系统与分层内容**：
  - `GET /api/v1/fs/ls`、`GET /api/v1/fs/tree`、`GET /api/v1/fs/stat`、`POST /api/v1/fs/mkdir`、`POST /api/v1/fs/mv`、`DELETE /api/v1/fs`
  - `GET /api/v1/content/abstract`（L0）、`GET /api/v1/content/overview`（L1）、`GET /api/v1/content/read`（L2）、`POST /api/v1/content/write`
- **检索与会话**：`POST /api/v1/search/find`、`POST /api/v1/search/search`（`mode="context"` 替代已废弃的 `/api/v1/search/recall`）、`POST /api/v1/search/grep`、`POST /api/v1/search/glob`、`POST /api/v1/sessions`、`POST /api/v1/sessions/{id}/messages`、`POST /api/v1/sessions/{id}/commit`。
- **多版本快照与 Wiki 编译**：`POST /api/v1/snapshot/commit`、`GET /api/v1/snapshot/log`、`POST /api/v1/snapshot/restore`、`GET /api/v1/snapshot/diff`、`POST /api/v1/compile`、`GET /api/v1/tasks/{task_id}`。

## 4. 四大 Agent 跨平台探测与注入规范

| Agent | 配置文件路径 | 接入形态 | 关键兼容细节 |
| :--- | :--- | :--- | :--- |
| **Claude Code** | `~/.claude.json`（MCP）+ `~/.claude/settings.json`（Hooks） | MCP + Hooks | SessionStart 注入限制 9500 bytes；扫描 `CLAUDE.md` 冷启动导入 |
| **Codex** | `~/.codex/config.toml` | MCP + Hooks | 使用 `toml_edit` 无损保留注释；提示首次启动 `"Hooks need review"` |
| **pi** | `~/.pi/extensions/openviking-bridge/` | 原生扩展桥接 | 写入轻量化 `index.js` + `manifest.json` 桥接 `ov-bridge`；Windows 采用目录拷贝规避软链权限 |
| **Antigravity（反重力）** | `~/.gemini/config/mcp_config.json` | MCP-Only | 避开已废弃的 `~/.gemini/antigravity/`；可选写入项目协作规则提示词 |

所有写入均通过 `atomic_write` 原子落盘（Windows 下支持杀软/IDE 短暂占用自动重试），并在修改前备份至 `~/.openviking/backups/<agent_id>/<timestamp>.bak`，支持一键卸载回滚。

## 5. Dreaming 后台整理流水线（Stage-then-Commit）

1. **Harvest（采集）**：拉取远端目录树、`viking://~/resources/inbox/` 与本地 SQLite 召回频次统计。
2. **Analyze（只读分析）**：并行运行 Inbox 分拣、冲突合并（仅限可变类 `profile`/`preferences`/`entities`，跳过不可变类与 `protected_rule_globs`）、冷热升降级/归档、Wiki 编译 4 个分析器。
3. **Stage（暂存）**：连同目标 URI 的 SHA-256 `base_hash` 存入 `~/.openviking/desktop.db` 的 `pending_memory_changes` 表，推送托盘红点与审核台。
4. **Commit（原子提交）**：用户审批（可在线编辑终稿）后，调用 `POST /api/v1/snapshot/commit` 创建快照 → 校验 `remote_hash == base_hash`（防并发覆盖）→ 批量写入/移动/触发 `ov compile` → 记录 `snapshot_id` 支持随时 `POST /api/v1/snapshot/restore` 一键回滚。
