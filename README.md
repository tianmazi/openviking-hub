# OpenViking Hub (`openviking-hub`)

跨平台（**macOS / Windows（含 WSL）/ Linux**）的 **OpenViking** 本地桌面控制平面、零依赖 MCP/Hooks 桥接引擎与 **Dreaming 记忆治理控制台**，基于 **Tauri v2 + Rust 3-Crate Workspace + React/TypeScript/Tailwind CSS** 构建。

- 仓库主页：[github.com/tianmazi/openviking-hub](https://github.com/tianmazi/openviking-hub)
- 安装包下载：[GitHub Releases](https://github.com/tianmazi/openviking-hub/releases)
- 完整使用手册：[docs/USER_GUIDE.md](docs/USER_GUIDE.md)
- 架构与 [OpenViking 官方 HTTP API](https://docs.openviking.ai/en/api/01-overview) 对齐规范：[docs/DESIGN.md](docs/DESIGN.md)

## 核心特性

- **远端多实例管理与实时观测**：托管 `~/.openviking/ovcli.conf` 与 `~/.openviking/profiles.json`，支持新增/编辑/切换多套远端 Profile（User Key 双头鉴权 `Authorization: Bearer` + `X-API-Key`）、实时连通性与延迟测试，以及远端 `observer/system`（Queue / VikingDB）观测面板与实时记忆信息流拓扑动效。
- **多 Agent 桌面端 & CLI 统一接入、WSL 注入与一键回滚**：自动探测并配置 **Claude Code**（MCP + Hooks）、**Codex（Desktop App + CLI）**（`$CODEX_HOME/config.toml`，`toml_edit` 无损保留注释）、**pi**（原生扩展桥接）与 **Antigravity（反重力 2.0 Desktop App + IDE + CLI）**（全局 `~/.gemini/config/mcp_config.json` + `~/.gemini/config/hooks.json` 双文件注入）；支持 Windows WSL 发行版跨 UNC 路径一键注入（强制 LF 换行）及原生记忆文件（`CLAUDE.md`、`AGENTS.md` 等）一键冷启动导入，修改前自动备份至 `~/.openviking/backups/`。
- **纯 Rust 单文件桥接（`ov-bridge`）**：作为 Tauri `externalBin` Sidecar 随安装包分发，启动时自动释放至 `~/.openviking/bin/`，同时承担 `stdio MCP Server`（含高分命中自动 L0 → L1 渐进式展开、**YAML Frontmatter 血统元数据写入**、**IFC `<openviking_memory_reference readonly="true">` 数据边界防劫持**）与生命周期 `Hook`（`session-start` 位置偏置结构化索引头 / `user-prompt-submit` / `stop`），彻底摆脱宿主机 Node.js、`npx.cmd`、Bash 或 Python 依赖。
- **内置记忆治理策略引擎（`~/.openviking/policy.toml`）**：
  - **作用域路由（Scope Router）**：`RESOURCE_POLICY.md` > 手动目录映射 > Git Remote 自动识别 > `viking://~/resources/inbox/` 兜底，支持可视化维护 `workspace_mappings` 与 `protected_rule_globs`（`CLAUDE.md`、`AGENTS.md` 只读保护）。
  - **分层注入与六类记忆配额**：按 Agent 配置 `L0/L1/L2` 默认召回层级与最大 Token 预算，并可视化配置 `profile` / `preferences` / `entities` / `events` / `cases` / `patterns` 的独立配额与 `mutable` / `immutable` 策略。
  - **一键隐私模式**：系统托盘与侧边栏均支持秒切只读隐私模式，实时拦截自动捕获与写入。
- **Dreaming 后台整理与 PR-Style 审核台**：支持 Cron 定时与 `SessionThreshold` 累计会话数触发，支持接入**本机 CLI Agent（`claude -p` / `codex exec`）或 OpenAI 兼容 LLM 反思器**进行语义冲突消解与 Inbox 智能项目归类（带纯程序确定性降级与 **Write-Gate 晋升门禁**），采用 **Stage-then-Commit** 流水线，提供逐行 `+`/`-` 双列行号 Diff、审阅进度条、键盘流审批（`J/K/A/X/E/⌘↵`）、在线编辑终稿、SHA-256 `base_hash` 乐观锁防并发漂移、Commit 前自动创建 Snapshot（`POST /api/v1/snapshot/commit`）及一键回滚（`POST /api/v1/snapshot/restore`）。

## 工程结构

```text
openviking-hub/
├── Cargo.toml                         # 3-Crate Workspace 根配置
├── docs/DESIGN.md                     # 完整架构与 OpenViking REST API 对齐规范
├── docs/USER_GUIDE.md                 # 终端用户安装与使用指南
├── crates/
│   ├── ov-core/                       # 共享核心库（REST 客户端、策略引擎、4 Agent + WSL 适配器、Dreaming + SQLite）
│   └── ov-bridge/                     # 纯 Rust 单文件 Sidecar（mcp / hook / status）与全链路 E2E 测试
├── src-tauri/                         # Tauri v2 桌面端外壳（系统托盘、Sidecar 释放、Cron/阈值调度、27 个 IPC 命令）
├── src/                               # React + TypeScript + Tailwind CSS 三栏控制台、记忆信息流拓扑与 Dreaming 审核台
├── tests/e2e/                         # 前端生产包与 Vite Preview 端到端冒烟测试
└── .github/workflows/release.yml      # macOS / Windows / Linux 四平台 CI 与自动发布流水线
```

## 快速开始与测试

### 1. 运行 Rust 单元测试与全链路 E2E 测试

```bash
cargo test -p ov-core -p ov-bridge
```

> `crates/ov-bridge/tests/e2e_bridge_and_dreaming.rs` 会启动本地 Mock OpenViking HTTP 服务并拉起真实编译的 `ov-bridge` 子进程，完整验证 MCP `recall`（含 L0→L1 自动展开）、受保护规则拦截、Dreaming 乐观锁冲突检测与 Snapshot 一键回滚。

### 2. 编译纯 Rust 桥接二进制（`ov-bridge`）

```bash
cargo build --release -p ov-bridge
```

### 3. 前端开发预览、生产构建与前端 E2E 测试

```bash
npm install
npm run dev       # 独立浏览器预览（内置 Mock 数据）
npm run build     # TypeScript 类型检查 + Vite 生产构建
npm run test:e2e  # 前端生产包 E2E 冒烟测试
```

### 4. 启动与打包 Tauri v2 桌面端

```bash
npm run tauri dev
npm run tauri build
```

推送 `v*` 标签（例如 `git tag v0.2.1 && git push origin v0.2.1`）会自动触发 GitHub Actions 构建 macOS（Apple Silicon + Intel）、Windows 与 Linux 四平台安装包并发布至 [GitHub Releases](https://github.com/tianmazi/openviking-hub/releases)。

### Agent capture queue (foundation)

The bridge captures textual user/assistant transcript records, filters injected references and common credential markers, and stores deduplicated messages in the active server profile's SQLite outbox. Stop, PreCompact and SessionEnd enqueue work; a short-lived sync process sends it to a reused OpenViking session and commits each delivered batch. Run `ov-bridge sync` to retry a connection failure and `ov-bridge doctor` to inspect pending/sent/uncertain counts. An uncertain remote write is not replayed automatically. Session commit acknowledgement is not proof that background memory extraction finished.

Git auto-routing now uses a hash of the normalized remote. Existing projects may retain their old resource directory with an explicit workspace mapping or RESOURCE_POLICY.md. The first sync uses the active profile only. Runtime protocol compatibility and real-server behavior should be checked before enabling capture for sensitive repositories; the simple credential filter is not a comprehensive DLP system.
