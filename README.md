# OpenViking Desktop

跨平台（**macOS / Windows（含 WSL）/ Linux**）的 **OpenViking** 本地桌面控制平面、零依赖 MCP/Hooks 桥接引擎与 **Dreaming 记忆治理控制台**，基于 **Tauri v2 + Rust 3-Crate Workspace + React/TypeScript/Tailwind CSS** 构建。

完整使用手册请参见 [docs/USER_GUIDE.md](docs/USER_GUIDE.md)，详细架构与 [OpenViking 官方 HTTP API](https://docs.openviking.ai/en/api/01-overview) 对齐规范请参见 [docs/DESIGN.md](docs/DESIGN.md)。

## 核心特性

- **远端多实例管理与实时观测**：托管 `~/.openviking/ovcli.conf` 与 `~/.openviking/profiles.json`，支持新增/编辑/切换多套远端 Profile（User Key 双头鉴权 `Authorization: Bearer` + `X-API-Key`）、实时连通性与延迟测试，以及远端 `observer/system`（Queue / VikingDB）观测面板。
- **多 Agent 统一接入、WSL 注入与一键回滚**：自动探测并配置 **Claude Code**（MCP + Hooks）、**Codex CLI**（`toml_edit` 无损保留注释）、**pi**（原生扩展桥接）与 **Antigravity（反重力）**（`~/.gemini/config/mcp_config.json`）；支持 Windows WSL 发行版跨 UNC 路径一键注入（强制 LF 换行）及原生记忆文件（`CLAUDE.md`、`AGENTS.md` 等）一键冷启动导入，修改前自动备份至 `~/.openviking/backups/`。
- **纯 Rust 单文件桥接（`ov-bridge`）**：桌面端启动时自动释放至 `~/.openviking/bin/`，同时承担 `stdio MCP Server`（含高分命中自动 L0 → L1 渐进式展开）与生命周期 `Hook`（`session-start` / `user-prompt-submit` / `stop`），彻底摆脱宿主机 Node.js、`npx.cmd`、Bash 或 Python 依赖。
- **内置记忆治理策略引擎（`~/.openviking/policy.toml`）**：
  - **作用域路由（Scope Router）**：`RESOURCE_POLICY.md` > 手动目录映射 > Git Remote 自动识别 > `viking://~/resources/inbox/` 兜底，支持可视化维护 `workspace_mappings` 与 `protected_rule_globs`（`CLAUDE.md`、`AGENTS.md` 只读保护）。
  - **分层注入与六类记忆配额**：按 Agent 配置 `L0/L1/L2` 默认召回层级与最大 Token 预算，并可视化配置 `profile` / `preferences` / `entities` / `events` / `cases` / `patterns` 的独立配额与 `mutable` / `immutable` 策略。
  - **一键隐私模式**：系统托盘与侧边栏均支持秒切只读隐私模式，实时拦截自动捕获与写入。
- **Dreaming 后台整理与 PR-Style 审核台**：支持 Cron 定时与 `SessionThreshold` 累计会话数触发，采用 **Stage-then-Commit** 流水线（Inbox 分拣、语义去重合并、高频跨 Agent 晋升、冷记忆归档、Wiki 编译），提供逐行 `+`/`-` 彩色 Diff、键盘流审批（`J/K/A/X/E/⌘↵`）、在线编辑终稿、SHA-256 `base_hash` 乐观锁防并发漂移、Commit 前自动创建 Snapshot（`POST /api/v1/snapshot/commit`）及一键回滚（`POST /api/v1/snapshot/restore`）。

## 工程结构

```text
openviking-desktop/
├── Cargo.toml                         # 3-Crate Workspace 根配置
├── docs/DESIGN.md                     # 完整架构与 OpenViking REST API 对齐规范
├── crates/
│   ├── ov-core/                       # 共享核心库（REST 客户端、策略引擎、4 Agent + WSL 适配器、Dreaming + SQLite）
│   └── ov-bridge/                     # 纯 Rust 单文件 Sidecar（mcp / hook / status）与全链路 E2E 测试
├── src-tauri/                         # Tauri v2 桌面端外壳（系统托盘、Sidecar 释放、Cron/阈值调度、27 个 IPC 命令）
├── src/                               # React + TypeScript + Tailwind CSS 三栏控制台与 Dreaming 审核台
├── tests/e2e/                         # 前端生产包与 Vite Preview 端到端冒烟测试
└── .github/workflows/release.yml      # macOS / Windows / Linux 三端 CI 与自动发布流水线
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

推送 `v*` 标签（例如 `git tag v0.1.0 && git push origin v0.1.0`）会自动触发 GitHub Actions 构建 macOS（Apple Silicon + Intel）、Windows 与 Linux 三端安装包并发布至 GitHub Releases。
