# OpenViking Desktop

跨平台（**macOS / Windows（含 WSL）/ Linux**）的 **OpenViking** 本地桌面控制平面、零依赖 MCP/Hooks 桥接引擎与 **Dreaming 记忆治理控制台**，基于 **Tauri v2 + Rust 3-Crate Workspace + React/TypeScript/Tailwind CSS** 构建。

详细架构与 API 对齐规范请参见 [docs/DESIGN.md](docs/DESIGN.md)。

## 核心特性

- **多 Agent 统一接入与一键回滚**：自动探测并配置 **Claude Code**（MCP + Hooks）、**Codex CLI**（`toml_edit` 无损保留注释）、**pi**（原生扩展桥接）与 **Antigravity（反重力）**（`~/.gemini/config/mcp_config.json`），支持 Windows WSL 发行版自动扫描；修改任何配置前自动备份至 `~/.openviking/backups/`。
- **纯 Rust 单文件桥接（`ov-bridge`）**：将 `stdio MCP Server` 与生命周期 `Hook`（`session-start` / `user-prompt-submit` / `stop`）打包为单个 Rust 二进制，彻底摆脱宿主机 Node.js、`npx.cmd`、Bash 或 Python 依赖。
- **内置记忆治理策略引擎（`~/.openviking/policy.toml`）**：
  - **作用域路由（Scope Router）**：`RESOURCE_POLICY.md` > 手动映射 > Git Remote 自动识别 > `viking://~/resources/inbox/` 兜底，并对 `CLAUDE.md`、`AGENTS.md` 等人工强约束文件启用只读保护。
  - **分层注入与 Token 预算**：按 Agent 独立配置 `L0/L1/L2` 默认召回层级、会话启动上限（如 Claude Code 9500 bytes 限制）与单次最大 Token 预算。
  - **一键隐私模式**：系统托盘与侧边栏均支持秒切只读隐私模式，实时拦截所有自动捕获与写入。
- **Dreaming 后台整理与 PR-Style 审核台**：采用 **Stage-then-Commit** 流水线——只读分析产出待审变更并记录远端 SHA-256 `base_hash`，支持 `J/K/A/X/E/⌘↵` 键盘流审批、右侧直接编辑终稿、证据链查看、Commit 前自动调用 `POST /api/v1/snapshot/commit` 创建快照以及一键回滚（`POST /api/v1/snapshot/restore`）。

## 工程结构

```text
openviking-desktop/
├── Cargo.toml                 # 3-Crate Workspace 根配置
├── docs/DESIGN.md             # 完整架构与 OpenViking REST API 对齐规范
├── crates/
│   ├── ov-core/               # 共享核心库（REST 客户端、策略引擎、跨平台适配器、Dreaming + SQLite）
│   └── ov-bridge/             # 纯 Rust 单文件 Sidecar（ov-bridge mcp / hook / status）
├── src-tauri/                 # Tauri v2 桌面端外壳（系统托盘、Cron 调度、22 个 IPC Commands）
├── src/                       # React + TypeScript + Tailwind CSS 三栏控制台与 Dreaming 审核台
└── .github/workflows/         # macOS / Windows / Linux 三端 CI 与发布流水线
```

## 快速开始

### 1. 运行 Rust 核心与桥接层测试

```bash
cargo test -p ov-core -p ov-bridge
```

### 2. 编译纯 Rust 桥接二进制（`ov-bridge`）

```bash
cargo build --release -p ov-bridge
```

### 3. 启动前端开发预览（支持浏览器独立预览）

```bash
npm install
npm run dev
```

### 4. 启动完整 Tauri v2 桌面端

```bash
npm run tauri dev
```
