# OpenPencil for VSCode 插件设计（spec v2）

日期：2026-07-16（v2：吸收 Codex review 结论，见同目录 `2026-07-16-vscode-extension-codex-review.md`）
状态：待实现
位置：`openpencil/packages/op-vscode/`（新目录）

## 目标

一个 VSCode 系插件，让 VS Code / Cursor / Trae / Windsurf 等 IDE 的用户能够：

1. 在 IDE 内用**完整的 OpenPencil 编辑器**打开、编辑 `.op` 设计文件；
2. 用 **IDE 自带的 AI 聊天/Agent** 通过 MCP 操作设计文档 —— AI 能力主路线；
3. 在支持 `vscode.lm` 的环境（VS Code + Copilot）额外获得 `@openpencil` chat participant；
4. 由 AI 驱动从 `.op` 设计生成前端代码（op-codegen 协议）。

## 需求决策记录

| 决策点 | 结论 |
|---|---|
| AI 路线 | 双路线：IDE 聊天经 MCP 为主，`vscode.lm` 入口为辅（不可用时静默隐藏） |
| 编辑深度 | 完整编辑器（嵌入 op-host-web wasm） |
| 运行时分发 | 平台专属 vsix 内置 `op-host-web-server` + `op` CLI + web-bundle/CanvasKit 资产 |
| 文档模型 | **每个打开的 .op 文档一个 daemon**（动态端口）；不做多文档 daemon 改造 |
| Rust 改动 | 承认需要一个**小型 Rust 桥接工作包**（见下），不再假设纯 TS 胶水 |
| 目标 IDE | VS Code、Cursor、Trae、Windsurf 及其它 VSCode 系（MCP 适配表） |

## 总体架构

```
┌─ VSCode / Cursor / Trae / Windsurf ────────────────────────────┐
│  ┌─ 插件 (TypeScript) ─────────────────────────────────────┐   │
│  │  DaemonPool（每文档一进程） McpProxy（稳定端口）          │   │
│  │  CustomEditorProvider (.op)  McpConfigurator  AiPanel   │   │
│  └───────┬──────────────────────────┬──────────────────────┘   │
│          │ webview(iframe+postMessage桥)   │ 反向代理            │
└──────────┼──────────────────────────┼──────────────────────────┘
           ▼                          ▼
  op-host-web-server ×N（每文档一个，动态端口，token 鉴权）
  ├── wasm 编辑器静态服务（web-bundle + CanvasKit）
  └── /mcp + /api（loopback + per-instance token）
           ▲
  IDE AI Agent ──► http://127.0.0.1:<稳定端口>/mcp（McpProxy 转发到活动文档的 daemon）
```

核心变化（相对 v1）：

- **daemon 二进制换成 `op-host-web-server`**（GL-free 无 GUI，本就为此存在）；插件直接
  spawn 它，不经 `op start --web`（后者会拉桌面版并强制开浏览器）。
- **每文档一个 daemon**：现有 daemon 严格单文档（单 `EditorState` + `current_path`），
  不改造；插件为每个打开的 `.op` custom editor 拉一个实例，维护 文档→端口 映射。
- **McpProxy 提供稳定 MCP 端点**：插件内置一个 TS 实现的本地 HTTP 反向代理，端口稳定并
  一次性写入各 IDE 的 MCP 配置；请求转发到**当前活动编辑器**对应的 daemon。避免 IDE
  MCP 配置随标签切换反复重写。
- **文件读写归插件**：保存时插件经桥接取文档字节，用 VS Code FS API 写盘（原子写、
  兼容 Remote/虚拟文件系统），不使用 daemon 的 `current_path` 全局保存路径。

## Rust 桥接工作包（前置依赖，量小但必须）

1. **wasm ⇄ 宿主消息桥**（op-host-web）：新增 `postMessage` 协议——
   - `dirty-changed{generation, revision}` 事件；`open-document{bytes}→{generation}`
     （每次 open/revert 使 **generation 自增**——`replace_document` 会把 revision 计数
     清零，revision 必须与 generation 联合才能唯一标识一个状态）；
   - `snapshot{purpose}→{bytes, generation, revision}`：**原子**地完成「flush 未推送
     编辑 + 导出字节 + 记录快照标识」，供保存/backup 使用（分离的 flush/export 两步
     会让写盘期间的新编辑被误标为已保存）；
   - `save-committed{generation, revision}`：宿主写盘成功后回传；generation 与当前不符
     的**过期确认直接丢弃**（防 revert/重开后延迟到达的确认污染新文档的已保存基线），
     匹配则调参数化的 `mark_saved_revision(revision)`。save/revert/reopen 在宿主侧
     串行化，竞态场景（写盘中 revert、连续快速保存）列入 Rust 测试。
   所有消息校验 `event.source`/`origin`。
2. **同步并发控制**（op-host-web + daemon）：现状 400ms 拉取会整体替换 wasm 文档、
   推送无条件写，编辑中的本地改动可被 MCP 更新静默覆盖。修订为：
   - **拉取门控**：wasm 存在未推送本地编辑时暂不应用远端文档（挂起直到本地推送完成）；
   - **条件推送**：`POST /api/mcp/document` 增加 `base_version`，版本不匹配拒绝写入
     （daemon 侧对 MCP 与浏览器写入统一生效，消灭 last-writer-wins）；
   - **冲突路径**：推送被拒时**提示用户二选一**（以本地覆盖远端 / 接受远端并把本地版
     本另存为副本），两个版本都不静默丢弃。自动重放需要命令日志/三方合并，wasm 现无
     此能力，列为 post-MVP（可复用 op-opmerge）。竞态场景（编辑中 MCP 写入、双向同
     时写）列入 Rust 测试。
   - **条件推送不设大小上限**：冲突后的重推与门控清除一律走 `snapshot`/flush 通道
     （不受 2 MiB 周期推送上限约束），否则超大 dirty 文档在 MCP 变更到来时会永久卡在
     拉取门控上。
3. **daemon 启动契约**（op-host-web-server）：`--port 0`（OS 分配）+ 启动后向 stdout
   输出 JSON（端口、shutdown token、版本）；`--file <path>` 初始加载；绝不打开浏览器；
   `sync-reset` 仅在首次挂载执行（防第二次挂载重载丢状态）；**父进程消亡自杀**——
   stdin 由插件持管道，读到 EOF（extension host 正常退出或崩溃均触发）即优雅退出，
   杜绝孤儿 daemon（现状：daemon 会一直 accept 直到收到带 token 的 shutdown）。
4. **loopback 鉴权 + token 引导**：per-instance 随机 token，`/api/*` 与 `/mcp` 全部
   校验（现状仅 shutdown 校验，页面首个请求 `sync-reset` 完全裸奔）；CORS 从 `*`
   收紧为显式 origin 白名单。token 到达页面的引导链：extension（stdout 握手获得）→
   webview 外层 shell → iframe `postMessage`（严格校验目标 origin）→ 页面/wasm 把
   token 附到之后所有 `/api` 请求 header；页面在收到 token 前不发任何 API 请求。
   token 全程只存在于内存，不落盘、不进 URL query（防日志泄漏）。

以上都在现有 crate 内做加法，不动架构；每项在 plan 里对应独立任务与测试。

## 组件明细

### DaemonPool

- 每个打开的 `.op` custom editor 对应一个 `op-host-web-server` 子进程（动态端口、
  独立 token）；进程句柄由插件持有（非 detached），插件持有其 stdin 管道——正常
  deactivate 显式关停，extension host 崩溃时 stdin EOF 触发 daemon 自杀（启动契约
  第 3 条），两条路径都不留孤儿。
- 启动握手：读 stdout JSON（端口/token/版本），版本与插件内置资产校验，防 bundle skew。
- 就绪检查 = `/mcp` ping **且**编辑器首页可加载（daemon 缺 bundle 时会降级为帮助页，
  仅 ping 不充分）。
- 崩溃：webview 显示重连页，自动重启一次并经桥接恢复未保存文档（backup 机制见下）；
  再失败通知用户并附诊断（stdout/stderr 采集到输出通道，不再 `spawn_null` 丢弃）。
- 端口冲突由 `--port 0` 天然规避；不使用机器级全局 manager 文件。

### CustomEditorProvider（`.op`）

- 注册为 `.op` 默认编辑器。webview 内为 daemon 页面的 iframe（`asExternalUri` 解析
  端口转发，兼容 Remote-SSH/WSL/Dev Containers），外层薄 JS 与 iframe 通过
  `postMessage` 桥通信（双向校验 `event.source`/`origin`），再经 `acquireVsCodeApi`
  上报扩展；webview CSP 的 `frame-src`/`connect-src` 仅允许该 daemon origin。
  token 引导链见 Rust 工作包第 4 条。
- **dirty**：桥的 `dirty-changed` 事件驱动 VS Code 标签 dirty 状态；MCP 编辑同样经
  daemon→wasm 同步触发 dirty。
- **save / save-as**：`snapshot{purpose:save}` 原子取 `{bytes, revision}` → VS Code FS
  原子写盘 → `save-committed{revision}`（写盘期间的新编辑不会被误标已保存）。
  **revert**：重新 `open-document(磁盘字节)`。
- **backup（hot-exit）**：`backupCustomDocument` 走同一 `snapshot{purpose:backup}`
  通道写备份文件（不触发 `save-committed`）。
- **外部磁盘变更**：文件 watcher 检测；编辑器不 dirty 则自动重载，dirty 则提示三选
  （重载丢弃/保留内存版/另存）。
- **undo/redo**：wasm 画布 undo 栈为唯一权威（webview 聚焦时快捷键由 wasm 消费）；
  不接 VS Code 的 custom editor edit 栈（MVP 明确不做步进级 undo 集成）。
- 同一文件被第二个编辑器打开时复用同一 daemon 的同一 webview（`retainContextWhenHidden`
  + 单实例策略），杜绝双视图 last-writer-wins。

### McpProxy + McpConfigurator

- McpProxy：TS 本地 HTTP 服务，端口取自设置（默认自动分配后持久化到 workspace 状态），
  把 `/mcp` 转发到活动 `.op` 编辑器的 daemon；无活动文档时返回结构化错误
  （提示 AI「先让用户打开一个 .op 文件」）。
- **两级凭据**：daemon 的 per-instance token 只存在于插件进程内，由 McpProxy 在转发时
  注入，**永不落盘**、随 daemon 重启自动轮换；McpProxy 自身不设 token —— 仅绑定
  loopback，校验 `Origin`/`Host` 头拒绝浏览器跨站请求（防 DNS rebinding），因此 MCP
  配置文件里只有 URL、没有 secret，多文档切换与 daemon 重启都不会使配置失效。
- McpConfigurator：命令「OpenPencil: Configure AI (MCP)」+ 首次激活提示。适配表按
  IDE（探测 `vscode.env.appName` + 特征 API 双重判断）写入：VS Code `.vscode/mcp.json`、
  Cursor `.cursor/mcp.json`、Trae/Windsurf 各自路径；每个适配器带版本化 schema 与
  「是否需要 reload」标记。写入前 diff 确认；处理 JSONC/注释/只读/多根 workspace/
  不受信 workspace 等失败场景（见错误表）。
- 附带「Install AI Skill」命令：Cursor 走现有 `op install`；Trae/Windsurf 由插件把
  同一 skill 内容写入各自的规则文件路径（给 `op install` 加新 target 为 post-MVP）。

### Codegen（AI 驱动）

- 事实澄清：`op codegen:plan/submit` 需要**调用方提供** plan/chunk JSON —— codegen
  本质是 AI 协议，不是纯命令。
- 主路线：openpencil-skill 已含 codegen 工作流指引，IDE Agent 经 MCP 走完整协议。
- 插件命令「Generate Code from Design」：在有 `vscode.lm` 的环境用它生成 plan/chunk
  并调 `op codegen:*` 编排（进度 UI + 可取消 + 产物目录冲突确认）；无 `vscode.lm` 时
  该命令引导用户到 IDE 聊天（预填提示词），不做假按钮。
- plan 存储是进程内存、30 分钟过期：编排器负责重试/过期重建/清理。

### AiPanel（vscode.lm 分支）

- 仅当 `vscode.lm` + Chat API 存在时注册 `@openpencil`，否则静默隐藏。
- 流程：用户请求 + 设计上下文（MCP `read_nodes`）→ IDE 模型 → batch_design 操作
  （经 MCP `batch_design` 写回，工具名以 `mcp_serve.rs` 注册名为准）。

## 安全模型

- daemon 仅绑定 loopback；所有 `/api` 与 `/mcp` 请求校验 per-instance token；token 经
  启动 stdout 交付插件，仅在插件进程内存与 McpProxy 转发 header 中存在，不写入任何
  配置文件（见「两级凭据」）。
- CORS 白名单：webview origin + 显式配置；拒绝其它 origin（防 DNS rebinding /
  恶意本地页面驱动设计文档写任意路径）。
- MCP `save_document` 等可写路径的工具在 daemon 侧限制到 workspace 目录内。
- 不受信 workspace：显示「受限模式」占位页（说明 + 一键信任入口），不拉 daemon、
  不写 MCP 配置、不提供预览（唯一渲染路径依赖 daemon，静态降级明确不做，故不承诺
  只读预览）。

## 远程与平台矩阵

- 远程（SSH/WSL/Dev Containers）：daemon 跑在 extension host 侧（即远端），webview 经
  `asExternalUri` 端口转发；IDE Agent 通常也在远端侧执行，MCP loopback 语义成立——
  该假设列入集成测试矩阵逐 IDE 验证；Codespaces Web 标记为 post-MVP。
- vsix 平台矩阵：darwin-arm64/x64、linux-x64/arm64、win32-x64；内置二进制含执行权限
  修复（macOS quarantine / Linux chmod）与签名/公证接入现有 release 流程。

## 错误处理汇总

| 场景 | 行为 |
|---|---|
| 打包资产缺失/损坏（daemon 在但 bundle 缺） | 就绪检查含编辑器页校验；失败给修复指引 |
| daemon 启动失败 | `--port 0` 规避端口冲突；失败通知 + stdout/stderr 诊断 |
| daemon 崩溃 | 重连页 + 自动重启一次 + backup 恢复；再失败交用户 |
| daemon 重启换端口 | McpProxy 内部重指向，IDE 配置无需变更 |
| extension host 重启 | daemon 随宿主退出；重开时从 backup/磁盘恢复 |
| 保存失败（只读/磁盘满/文件被删/外部修改） | VS Code FS 错误直传 + 外部变更三选提示 |
| MCP 配置写入（JSONC/只读/多根/不受信/需 reload） | 适配器逐项处理，diff 确认 |
| MCP 请求无活动文档 | 结构化错误提示 AI 引导用户打开文件 |
| codegen plan 过期/部分失败 | 编排器重建/重试/清理；产物写入前冲突确认 |
| `vscode.lm` 不存在 | AiPanel/codegen 编排静默降级 |
| 版本 skew | 启动握手校验 daemon 版本 vs 插件资产版本 |
| 卸载/升级 | deactivate 停 daemon、清 backup;提供「移除 MCP 配置/skill」命令 |

## 测试

- 单测（vitest）：McpConfigurator 各 IDE 适配器；DaemonPool 状态机（mock 子进程）；
  McpProxy 路由/无活动文档错误；save/backup/revert 桥接协议（mock webview）。
- Rust 侧：桥接工作包每项带 crate 内测试（flush-ack、token 鉴权、sync-reset 守卫）。
- 集成冒烟：真实 spawn `op-host-web-server` → 握手 JSON → 编辑器页可达 → token 生效
  （无 token 401）→ 打开示例 `.op` → flush+export 字节与磁盘一致。
- 手动矩阵：{VS Code, Cursor, Trae, Windsurf} × {本地, Remote-SSH/WSL} 验证
  「配置 MCP → IDE AI 改设计 → webview 实时可见 → 保存/undo/外部修改」；
  另验证同文件双开、>2 MiB 文档、hot-exit 恢复。

## 明确不做（MVP 之外）

- 多文档单 daemon 改造；VS Code 步进级 undo 集成（edit 栈）
- 同步冲突的自动重放/三方合并（op-opmerge 集成）——MVP 冲突走用户二选一
- Codespaces Web / 浏览器版 VS Code
- webview 离线/静态降级；AiPanel 流式画布预览；Figma 导入 UI
- 遥测（仅本地日志输出通道，无上报）
