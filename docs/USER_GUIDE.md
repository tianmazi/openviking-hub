# OpenViking Hub 使用指南

本指南面向终端用户，介绍如何安装 **OpenViking Hub（OpenViking Desktop）**、连接远端 OpenViking 服务、一键接入本地编程 Agent（含 Desktop App 与 CLI）、配置记忆治理策略以及使用 Dreaming 审核台。

---

## 1. 安装与首次启动

前往仓库 [Releases](https://github.com/tianmazi/openviking-hub/releases) 页面下载对应平台的安装包：

- **macOS（Apple Silicon / M 系列）**：下载 `*_aarch64.dmg`
- **macOS（Intel）**：下载 `*_x64.dmg`
- **Windows x64**：下载 `*_x64-setup.exe`（NSIS 安装器）或 `*_x64_en-US.msi`
- **Linux x64**：Ubuntu / Debian 下载 `*_amd64.deb`，Fedora / RHEL 下载 `*.rpm`，通用发行版下载 `*_amd64.AppImage`

> **首次启动说明**：
> - **macOS**：由于尚未做 Apple Developer 公证，首次双击若提示“无法验证开发者”，请在「系统设置 → 隐私与安全性」中点击“仍要打开”，或在终端执行 `xattr -cr "/Applications/OpenViking Desktop.app"`。
> - **Linux（NVIDIA + Wayland）**：如果遇到窗口白屏，可附带 `--disable-gpu` 参数启动。
> - **本地文件初始化**：首次启动后，应用会在 `~/.openviking/`（Windows 为 `%USERPROFILE%\.openviking\`）下自动初始化：
>   - `ovcli.conf`：当前生效的远端连接凭据（Unix 权限 `0600`）
>   - `profiles.json`：多套远端实例 Profile
>   - `policy.toml`：内置记忆治理策略配置
>   - `desktop.db`：本地 SQLite（召回事件流、Dreaming 队列与免归档列表）
>   - `bin/ov-bridge`（Windows 为 `ov-bridge.exe`）：自动释放的零依赖纯 Rust 桥接二进制

---

## 2. 连接远端 OpenViking 服务

1. 在左侧栏 **「远端实例 Profile」** 右侧点击设置图标，打开实例管理弹窗。
2. 填写以下信息：
   - **Profile ID / 显示名称**：如 `prod-vps` / `自建远端 OpenViking`
   - **Server URL**：你的 OpenViking 服务地址（如 `http://127.0.0.1:1933` 或托管服务域名）
   - **User API Key**：**请务必使用 User Key，不要使用 Root Key**（Root Key 无法直接读写普通用户记忆空间）。
   - **Account / User**（可选）：仅在服务端为 `trusted` 模式时需要填写。
3. 点击 **「测试连通性」** 确认返回绿色延迟后，点击 **「保存并切换」**。
4. 顶部状态灯显示绿色延迟（如 `18ms`）即表示连接就绪；所有本地 Agent 通过 `ov-bridge` 自动共享这一份凭据，无需逐个配置密钥。

---

## 3. 一键接入本地 Agent

切换到左侧 **「本地 Agent 接入」** 页面，应用会自动扫描本机已安装的编程 Agent（同时识别 Desktop App 与 CLI）：

| Agent | 接入方式 | 接入后注意事项 |
| :--- | :--- | :--- |
| **Claude Code** | 自动注入 `~/.claude.json`（MCP）与 `~/.claude/settings.json`（Hooks） | 启动注入自动限制在 9500 bytes 内，重启 Claude Code 会话即生效 |
| **Codex（Desktop App + CLI）** | 使用 `toml_edit` 无损合并全局 `$CODEX_HOME/config.toml`（默认 `~/.codex/config.toml`） | 桌面端与 CLI 共用全局配置；接入后请重启 Codex Desktop 并新建线程生效，CLI 首次启动选择 `Trust all and continue` |
| **pi** | 安装原生扩展至 `~/.pi/extensions/openviking-bridge/` | 自动拉取并注册 `openviking_*` 原生工具 |
| **Antigravity（反重力 2.0 Desktop / IDE / CLI）** | 同步注入全局 `~/.gemini/config/mcp_config.json` 与 `~/.gemini/config/hooks.json` | 接入后请在桌面端 **Customizations → Installed → MCP Servers** 点击 **Refresh** 或重启应用生效 |

### 备份回滚与原生记忆冷启动导入
- **安全回滚**：修改任何配置文件前，原文件会自动备份至 `~/.openviking/backups/<agent_id>/`。点击卡片上的 **「卸载并回滚」** 可一键清除 OpenViking 注入项。
- **冷启动导入**：如果检测到本地已有 `CLAUDE.md`、`AGENTS.md`、`MEMORY.md` 或 `GEMINI.md`，卡片下方会直接列出文件名，点击 **「导入远端」** 即可一键同步到 `viking://~/resources/inbox/` 等待归类。
- **Windows WSL 支持**：Windows 端会自动扫描已安装的 WSL 发行版，点击 **「一键注入 WSL」** 即可向 `\\wsl$\<Distro>\home\<user>\` 写入 LF 换行的 MCP 配置。

---

## 4. 浏览与编辑 `viking://` 记忆空间

在 **「viking:// 记忆浏览器」** 中：
1. 顶部可快速切换 **用户共享记忆**（`viking://~/memories/`）、**项目知识库**（`viking://resources/projects/`）和 **Inbox 待分拣区**（`viking://~/resources/inbox/`）。
2. 可在搜索框输入自然语言关键词，直接调用远端语义搜索（`POST /api/v1/search/find`）定位条目。
3. 点击 **「新建」** 可直接创建新的 Markdown 记忆；悬停条目可删除。
4. 右侧检查器支持切换 **L0（~100 tokens 摘要）/ L1（~2k tokens 概览）/ L2（完整原文）**；在 L2 标签页可直接在线编辑 Markdown 并点击 **「保存 L2」** 写回远端并刷新语义索引。

---

## 5. 配置记忆治理策略与隐私模式

在 **「记忆策略中心」** 修改后点击 **「保存策略」**（或底部未保存浮条中的 **「保存生效」**），配置会原子写入 `~/.openviking/policy.toml`，`ov-bridge` 在下一次调用时自动热加载，**无需重启任何 Agent**：

1. **作用域路由（Scope Router）**：
   - 优先级：**项目根目录 `RESOURCE_POLICY.md`** > **手动目录映射** > **Git Remote 自动识别** > **Inbox 兜底**。
   - 如需在某个项目单独指定远端 URI，可在项目根目录创建 `RESOURCE_POLICY.md`：
     ```markdown
     ---
     resource_uri: viking://resources/projects/my-service/
     ---
     ```
   - **规则只读保护**：列入 `protected_rule_globs`（默认含 `CLAUDE.md`、`AGENTS.md`、`.cursorrules`、`.agents/rules/**`）的文件会被强制禁止 AI 写入或 Dreaming 覆盖。
2. **分层注入与 Token 预算**：
   - 可分别为 Claude Code / Codex / pi / Antigravity 拖动滑块设置最大召回 Token 预算和默认召回层级（`L0`/`L1`/`L2`）。
   - 当默认层级为 `L0` 且命中相似度 ≥ `auto_expand_l1_threshold`（默认 `0.80`）时，`ov-bridge` 会自动展开高分条目的 `L1` 概览。
   - 可独立调整六类记忆（`profile` / `preferences` / `entities` / `events` / `cases` / `patterns`）的配额与 `mutable`（允许合并）/ `immutable`（仅追加归档）属性。
3. **一键隐私模式**：
   - 点击左下角 **「自动捕获已开启」** 或系统托盘菜单的 **「切换隐私模式」**，可瞬间切换为只读模式：所有 Agent 仍可正常召回，但任何写入/捕获操作会被实时拦截并记录在事件流中。

---

## 6. 使用 Dreaming 后台整理与审核台

Dreaming 采用 **Stage-then-Commit（先分析暂存，人工审批后再原子提交）** 机制，绝不静默篡改你的记忆：

1. **触发方式**：支持在策略中心选择 **Cron 定时**（默认每天凌晨 `0 3 * * *`）、**累计会话数阈值**（`SessionThreshold`），或在审核台/系统托盘随时点击 **「立即整理」**。
2. **审批工作流（全键盘流）**：
   - `J` / `K`：上下切换待审提案
   - `A`：批准并暂存（仅标记本地状态，尚未写入远端）
   - `X`：拒绝该提案
   - `E`：在右侧逐行 Diff 视图中直接在线编辑终稿（编辑状态下按 `⌘↵` / `Ctrl+↵` 直接批准）
   - `⌘↵` / `Ctrl+↵`：提交所有已批准项
3. **批量处理与安全兜底**：
   - 点击 **「批量批准安全项」** 可一键暂存所有 `Low Risk` 提案（如长期 0 命中的冷记忆归档、Wiki 编译），将精力集中在冲突合并和 Inbox 分拣。
   - 对不想被归档的条目，点击 **「永久保留 / 免打扰」** 加入保护列表。
   - 点击 **「提交已批准」** 时，系统会先调用远端 `POST /api/v1/snapshot/commit` 创建快照，再逐条校验 SHA-256 `base_hash`（若期间远端文件已被改动，会自动标记冲突拦截覆盖）。
4. **一键回滚**：点击右上角 **「历史与回滚」**，找到对应批次点击 **「回滚快照」**，即可通过 `POST /api/v1/snapshot/restore` 恢复提交前的完整状态。

---

## 7. 常用快捷键与实时记忆信息流观测

- **`⌘K` / `Ctrl+K`**：唤起全局命令面板，快速切换页面、切换隐私模式或立即执行 Dreaming 整理。
- **实时记忆信息流拓扑与事件流**：顶部展示本地 Agent 与远端 OpenViking 记忆中枢的动态光束拓扑（绿色代表记忆捕获写入、靛蓝代表召回注入、琥珀色代表隐私模式拦截），下方可查看远端 `observer/system`（Queue / VikingDB）状态、按 Agent 筛选近期 Recall/Capture 记录、命中层级（L0/L1/L2）、单次 Token 消耗与响应延迟。

---

## 8. 常见问题排查

- **Agent 启动后看不到 `openviking` 工具**：
  1. 确认 `~/.openviking/bin/ov-bridge`（Windows 为 `ov-bridge.exe`）存在且有可执行权限。
  2. 确认已在桌面端保存过有效的远端 Profile（`~/.openviking/ovcli.conf` 非空）。
  3. Antigravity 用户请在桌面端 **Customizations → Installed → MCP Servers** 手动点击 **Refresh**；Codex Desktop 用户请重启应用并新建线程；Claude Code / CLI 用户请重开终端会话。
- **Dreaming 提交时提示“远端漂移冲突”**：说明在你审批期间，其他 Agent 或手动编辑修改了同一条远端记忆；点击 **「立即整理」** 重新计算 Diff 即可安全合并。
- **手动测试桥接是否正常**：可在终端直接运行 `~/.openviking/bin/ov-bridge status`，查看返回的远端服务连通状态与当前隐私模式开关。
