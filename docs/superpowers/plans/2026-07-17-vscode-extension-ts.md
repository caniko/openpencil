# VSCode 插件 TS 本体 — 实施计划（Plan 2/3）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 `openpencil/packages/op-vscode/` 插件本体，消费 Plan 1 已落地的 Rust 契约：per-doc managed daemon、postMessage 桥、token 鉴权、McpProxy 稳定端点、MCP 一键配置、codegen/AiPanel 薄入口。

**Architecture:** 纯 TypeScript 插件（Node extension host）。核心分层：`daemon/`（进程与 HTTP 契约层，不依赖 vscode API，可单测）→ `session/`（PenSession 文档协议状态机，vscode API 经接口注入，可单测）→ `vscode/`（CustomEditorProvider/命令/UI 薄壳）。webview 只是一个装 iframe 的转发壳。

**Tech Stack:** TypeScript + esbuild（打包）+ Bun（脚本/测试运行器，`bun test`）+ oxlint/oxfmt（对齐 `packages/` 工具链）。运行时零 npm 依赖（JSONC 编辑用 `jsonc-parser` 一个例外，vendored 到 devDeps 并打进 bundle）。

**Plan 系列:** Plan 1（Rust 基座）已完成（17 commits，v0.8.2）。本文 = Plan 2。Plan 3 = 平台 vsix 打包/发布矩阵（本 plan 用 `openpencil.dev.daemonPath` 设置指向 cargo 构建产物，不涉打包）。

## 已落地契约（实现时以这些为准，不得凭记忆改写）

- **daemon 启动**：`op-host-web-server --serve-web --managed --port 0 [--file <path>] [--allow-origin <origin>]...`；stdout 首行握手 `{"ok":true,"port":<u16>,"token":"<hex32>","version":"<semver>"}`；插件持 stdin 管道，EOF 即 daemon 自杀；进程句柄非 detached。
- **鉴权**：managed 模式下除静态 GET（`/`、`/index.html`、`/pkg/*`、`/smoke/*`、`/canvaskit/*`、`/assets/*`）与 OPTIONS 外，一切请求需 `X-OpenPencil-Token: <token>`，否则 401 `{"ok":false,"error":"unauthorized"}`。
- **CORS**：仅回显 `--allow-origin` 白名单命中的 Origin；预检通告 `X-OpenPencil-Token, Content-Type`。
- **文档端点**：`GET /api/mcp/version`→**`{"version":N}`**（注意：无 `ok` 字段——web_canvas_server.rs:391 实测形态）；`POST /api/mcp/sync-reset` 幂等（二次 `{"ok":true,"skipped":true,...}`）；`POST /api/mcp/document` 带顶层 `baseVersion` 条件写（过期 409 `{"ok":false,"error":"version-conflict","version":N}`）；`POST /mcp` = MCP JSON-RPC（initialize/tools/list/tools/call）。
- **桥传输载体**：postMessage 两个方向承载的都是 **JSON 字符串**（Rust `BridgeInbound::parse(raw: &str)` / `event_*() -> String`）——TS 侧必须 `JSON.stringify` 出站、对 `typeof data === "string"` 的入站做 `JSON.parse`，对象直接丢弃。
- **就绪判定**：bundle 缺失时 `GET /` 返回 404 帮助页（页内恰含 `op_host_web.js` 字样——**不能**做字符串探测）；正确判定 = `GET /` 200 **且** `GET /pkg/op_host_web.js` 200。
- **桥协议**（`crates/op-editor-core/src/bridge_protocol.rs` 为唯一权威）：
  - 插件→页面（经 iframe postMessage）：`op-bridge/init{token}`、`op-bridge/open-document{json}`、`op-bridge/snapshot{purpose,requestId}`、`op-bridge/save-committed{generation,revision}`、`op-bridge/resolve-conflict{mode:"use-local"|"accept-remote",requestId}`
  - 页面→插件：`op-bridge/ready{generation,revision}`（**bootstrap reset 完成后才发**，收到前不得 open/snapshot）、`op-bridge/dirty-changed{generation,revision,dirty}`、`op-bridge/opened{generation}`、`op-bridge/snapshot-result{requestId,docJson,generation,revision}`、`op-bridge/snapshot-conflict{requestId,serverVersion}`、`op-bridge/sync-conflict{generation,revision,serverVersion}`、`op-bridge/conflict-resolved{requestId}`
- **契约义务（终审标注）**：AcceptRemote 分支页面会先发 `snapshot-result`（本地字节）再发 `conflict-resolved`——宿主**必须先把备份写盘成功**再继续处理；两个版本都不许丢。
- **编辑器页面**：daemon 根路径 `/` 服务 wasm 编辑器（bundle 缺失时是帮助页——就绪检查不能只 ping `/mcp`）。

## Global Constraints

- 目录 `openpencil/packages/op-vscode/`；`.ts` 文件名 **kebab-case**；单文件 ≤ **800 行**；源码注释英文。
- Conventional Commits：scope 一律 `vscode`（新 scope，plan 3 起沿用）。
- `daemon/` 与 `session/` 层 **禁止 import "vscode"**（单测门槛）；vscode API 只出现在 `src/vscode/` 与 `src/extension.ts`。
- 每任务收尾：`bun test` 全绿 + `bun run typecheck`（`tsc --noEmit`）+ `bun run lint`（oxlint）全过再 commit。
- 不受信 workspace：受限模式（不拉 daemon、不写配置、占位页）——spec §安全模型。

---

### Task 1: 包脚手架 + 构建/测试工具链

**Files:**
- Create: `packages/op-vscode/package.json`、`tsconfig.json`、`build.mjs`（esbuild）、`.vscodeignore`、`.oxlintrc.json`、`src/extension.ts`（空激活骨架）、`README.md`（3 行占位）
- Test: `src/smoke.test.ts`（工具链自证）

**Interfaces:**
- Produces: `bun run build`（esbuild → `dist/extension.js`，external: `vscode`）、`bun test`、`bun run typecheck`、`bun run lint` 四条命令全部可用。
- Produces: package.json `contributes`: customEditors（`openpencil.penEditor`，selector `*.op`，priority `default`——**注意 `supportsMultipleEditorsPerDocument` 不是 manifest 属性**，它属于 Task 7 的 `window.registerCustomEditorProvider` 第三参）、commands（`openpencil.configureMcp` / `openpencil.removeMcp` / `openpencil.installSkill` / `openpencil.removeSkill` / `openpencil.generateCode`）、**`chatParticipants`**（id `openpencil.assistant`，name `openpencil`——`createChatParticipant` 要求 manifest 条目对应）、configuration（`openpencil.dev.daemonPath`（string，默认 `""`：留空时按 `<workspace>/target/debug/op-host-web-server` 探测）、`openpencil.proxy.port`（number，默认 0=自动）、`openpencil.codegen.framework`（enum react/vue，默认 react））；`capabilities.untrustedWorkspaces: { supported: "limited", description: "Without trust, .op files show a read-only placeholder; no local daemon is started and no MCP config is written." }`（**"limited" 必须带 description**）；`engines.vscode: "^1.90.0"`（**1.85 的 @types 没有 `vscode.lm`/`vscode.chat`——1.90 才有**）。
- Produces: 测试类型环境——devDependencies 增 `@types/bun`，tsconfig `types: ["bun", "node"]`，测试文件 `import { test, expect } from "bun:test"`（node+vscode 的 types 配置过不了 bun test 的类型检查）。devDependencies `@types/vscode@~1.90`。
- Consumes: 无。

- [ ] **Step 1: 写 package.json**（完整内容按上述 contributes；`main: "./dist/extension.js"`；scripts: `build`/`watch`/`test`(bun test)/`typecheck`/`lint`；devDependencies: `@types/vscode@~1.90`、`@types/node@^20`、`@types/bun`、`typescript@^5`、`esbuild@^0.21`、`jsonc-parser@^3`、`oxlint`）
- [ ] **Step 2: 写 build.mjs**

```js
import esbuild from "esbuild";
const watch = process.argv.includes("--watch");
const ctx = await esbuild.context({
  entryPoints: ["src/extension.ts"],
  bundle: true,
  outfile: "dist/extension.js",
  external: ["vscode"],
  format: "cjs",
  platform: "node",
  target: "node18",
  sourcemap: true,
});
if (watch) await ctx.watch(); else { await ctx.rebuild(); await ctx.dispose(); }
```

- [ ] **Step 3: tsconfig（strict, moduleResolution bundler, `types: ["bun","node"]`——@types/vscode 是 @types 作用域自动纳入）+ 空 extension.ts（activate/deactivate 导出 + OutputChannel "OpenPencil" 创建）+ smoke.test.ts（`import { test, expect } from "bun:test"`；`expect(1+1).toBe(2)`）**
- [ ] **Step 4: 验证** — Run: `cd packages/op-vscode && bun install && bun run build && bun test && bun run typecheck && bun run lint`；Expected: 全过
- [ ] **Step 5: Commit** — `feat(vscode): scaffold extension package with build and test toolchain`

---

### Task 2: 桥协议 TS 编解码（`src/protocol/bridge.ts`）

**Files:**
- Create: `packages/op-vscode/src/protocol/bridge.ts`、`src/protocol/bridge.test.ts`

**Interfaces:**
- Produces（与 Rust `bridge_protocol.rs` 镜像；**测试向量从 Rust 测试逐字拷贝**，保证跨语言一致）：

```ts
export type BridgeOutboundToPage =
  | { type: "op-bridge/init"; token: string }
  | { type: "op-bridge/open-document"; json: string }
  | { type: "op-bridge/snapshot"; purpose: "save" | "backup" | "conflict-backup"; requestId: string }
  | { type: "op-bridge/save-committed"; generation: number; revision: number }
  | { type: "op-bridge/resolve-conflict"; mode: "use-local" | "accept-remote"; requestId: string };

export type BridgeInboundFromPage =
  | { type: "op-bridge/ready"; generation: number; revision: number }
  | { type: "op-bridge/dirty-changed"; generation: number; revision: number; dirty: boolean }
  | { type: "op-bridge/opened"; generation: number }
  | { type: "op-bridge/snapshot-result"; requestId: string; docJson: string; generation: number; revision: number }
  | { type: "op-bridge/snapshot-conflict"; requestId: string; serverVersion: number }
  | { type: "op-bridge/sync-conflict"; generation: number; revision: number; serverVersion: number }
  | { type: "op-bridge/conflict-resolved"; requestId: string };

/** Wire format is a JSON STRING on both directions (Rust parses &str and
 *  emits String) — non-string payloads are foreign traffic. */
export function encodeOutbound(msg: BridgeOutboundToPage): string; // JSON.stringify
/** null for foreign/malformed messages — mirror of BridgeInbound::parse's None. */
export function parseInboundFromPage(raw: unknown): BridgeInboundFromPage | null; // typeof raw !== "string" → null; JSON.parse 失败 → null
```

  `parseInboundFromPage` 按 `type` 分发 + 逐字段类型校验（数字字段用 `typeof x === "number" && Number.isSafeInteger(x) && x >= 0`——Rust 侧是 u64）；未知 type / 缺字段 → null。
- Consumes: 无。

- [ ] **Step 1: 写失败测试**（用例含：每种入站消息一条合法样本（字段名 camelCase 与 Rust event_* 输出一致，如 `{"type":"op-bridge/sync-conflict","generation":2,"revision":5,"serverVersion":12}`）；`{"type":"react-devtools"}` → null；非对象/字符串 → null；`snapshot-result` 的 `docJson` 含转义引号往返）
- [ ] **Step 2: `bun test src/protocol` 确认失败** — Expected: FAIL（模块不存在）
- [ ] **Step 3: 实现** — 按 Interfaces；无第三方依赖
- [ ] **Step 4: `bun test` 全绿**
- [ ] **Step 5: Commit** — `feat(vscode): bridge protocol codec mirroring the rust wire format`

---

### Task 3: DaemonClient — 进程包装 + 握手（`src/daemon/daemon-client.ts`）

**Files:**
- Create: `src/daemon/daemon-client.ts`、`src/daemon/daemon-client.test.ts`、`test/fixtures/fake-daemon.mjs`

**Interfaces:**
- Produces:

```ts
export interface DaemonHandshake { port: number; token: string; version: string }
export interface DaemonLogger { info(line: string): void; error(line: string): void }
export interface SpawnOptions {
  /** Full command line — tests inject [process.execPath, fixturePath]; prod
   *  injects [binaryPath]. Daemon args are appended after this prefix. */
  command: string[];
  filePath?: string;         // --file
  allowOrigin: string;       // webview origin for --allow-origin
  logger: DaemonLogger;
  handshakeTimeoutMs?: number; // default 10_000
  /** Version the extension expects (from its own manifest metadata); a
   *  mismatching handshake version logs a warning (never a failure — dev
   *  builds drift), and the warning must NOT include the token. */
  expectedVersion?: string;
}
export class DaemonClient {
  static async spawn(opts: SpawnOptions): Promise<DaemonClient>; // rejects on spawn error / handshake timeout / malformed handshake
  readonly handshake: DaemonHandshake;
  get baseUrl(): string;      // http://127.0.0.1:<port>
  get alive(): boolean;
  onExit(cb: (code: number | null) => void): void;
  /** Closes stdin (parent-death lease), resolves when the process exits;
   *  SIGKILL fallback after 3s. Promise-returning so disposeAll/deactivate
   *  can await the full shutdown chain. */
  dispose(): Promise<void>;
}
```

  实现要点：`const [exe, ...prefix] = opts.command; child_process.spawn(exe, [...prefix, "--serve-web","--managed","--port","0", ...file?["--file",file]:[], "--allow-origin", allowOrigin], { stdio: ["pipe","pipe","pipe"] })`（command-prefix 注入让测试能跑 `[process.execPath, fake-daemon.mjs]`）；stdout 按行缓冲读首行 JSON（**有界超时**——Plan 1 冒烟的已知 gap，这里必须做对）；握手行**不得**原样进 logger（含 token——记录时替换为 `token:<redacted>`），malformed 握手的报错信息同样只含前 32 字节且先做 token 脱敏；后续 stdout/stderr 行转发 logger 前**逐行做 token 脱敏**（`line.replaceAll(token, "<redacted>")`——daemon 诊断输出可能回显 URL/头部，redaction 覆盖每一条转发行，不只握手行）；`dispose()` 先 `child.stdin.end()`（触发 daemon stdin-EOF 自杀）、3 秒未退再 `kill("SIGKILL")`，进程退出后 resolve；`expectedVersion` 与握手 version 不一致 → logger.info 警告（不失败）。**握手前失败的统一清理**：`spawn()` 的每一条 reject 路径（超时、malformed 握手、握手前子进程早退、**stdout 在握手前关闭/出错**）都走同一个 `try/finally` 清理——kill 子进程并 await 退出（早退场景跳过 kill），绝不泄漏 managed daemon。fake-daemon 增加 `--close-stdout` 模式（写半行后 `process.stdout.end()` 且自身存活）供流错误路径测试；四条 reject 路径各有测试断言子进程已退出。
- Produces: `test/fixtures/fake-daemon.mjs` —— node 脚本，模拟契约：立即输出握手行（端口/token 来自 argv 注入以便断言）、stdin EOF 时 exit 0、支持 argv 开关模拟「不输出握手」「输出垃圾」「延迟 N ms」三种故障。
- Consumes: Task 1 工具链。

- [ ] **Step 1: 写失败测试**（真实 spawn fake-daemon：正常握手解析出 port/token/version；`--file` 与 `--allow-origin` 参数原样传递（fake-daemon 把 argv echo 进握手的扩展字段供断言）；**每条 reject 路径断言子进程已退出**——握手超时（fake 延迟 > timeout）、垃圾握手、握手前早退（fake 立即 exit 1）、握手前 stdout 关闭（fake `--close-stdout`）四种；转发行 token 脱敏断言（fake 在握手后 echo 一行含 token 的日志）；logger 捕获断言无 token 明文；dispose 后 fake 进程退出（stdin-EOF 路径，断言 exit code 0 而非被 kill）且 dispose promise resolve）
- [ ] **Step 2: 确认失败** — `bun test src/daemon`
- [ ] **Step 3: 实现**（无 vscode import；Node 标准库 only）
- [ ] **Step 4: `bun test` 全绿 + typecheck + lint**
- [ ] **Step 5: Commit** — `feat(vscode): daemon process client with bounded handshake and eof lease`

---

### Task 4: Daemon HTTP 契约层（`src/daemon/daemon-http.ts`）

**Files:**
- Create: `src/daemon/daemon-http.ts`、`src/daemon/daemon-http.test.ts`

**Interfaces:**
- Produces:

```ts
export class DaemonHttp {
  constructor(baseUrl: string, token: string, timeoutMs?: number); // default 8000; tests inject short values
  async version(): Promise<number>;                          // GET /api/mcp/version → {"version":N}（无 ok 字段！）；N 必须是非负安全整数（Number.isSafeInteger && >=0），否则 reject
  async ready(): Promise<boolean>;                           // GET / 200 AND GET /pkg/op_host_web.js 200（bundle 缺失时 / 是 404 帮助页且页内含 "op_host_web.js" 字样——绝不做 body 字符串探测）
  async getDocument(): Promise<string>;                      // GET /api/mcp/document → raw body
  async mcpRaw(body: string, extraHeaders?: Record<string,string>): Promise<{ status: number; headers: Record<string,string>; body: string }>; // POST /mcp passthrough (McpProxy 用)
}
```

  全部请求带 `X-OpenPencil-Token`；`fetch` 用 Node 18 全局 fetch + `AbortSignal.timeout(8000)`。**注意**：文档保存/打开不走这里（走桥的 snapshot/open-document——daemon 侧文档由页面推送，插件不直接 POST /api/mcp/document，避免绕过 SyncGate）。
- Consumes: Task 3 的 baseUrl/token。

- [ ] **Step 1: 写失败测试**（node `http.createServer` 桩：断言 token 头存在、无 token 时桩回 401 且方法 reject；`version()` 对 `{"version":7}` 返回 7、对 `{"version":-1}`/`{"version":1.5}`/`{"version":"7"}` reject；`ready()` 对「/ 404」返回 false、对「/ 200 但 /pkg/op_host_web.js 404」返回 false、对双 200 返回 true；超时路径用不响应的桩 + 注入的短 timeout）
- [ ] **Step 2-5: 红 → 实现 → 绿 → Commit** — `feat(vscode): daemon http contract layer with token and readiness probe`

---

### Task 5: PenSession — 文档协议状态机（`src/session/pen-session.ts`）

这是插件的心脏：把桥消息流转成 VS Code 文档生命周期动作。**不 import vscode**——所有宿主副作用经注入的 `SessionHost` 接口，状态机可全量单测。

**Files:**
- Create: `src/session/pen-session.ts`、`src/session/pen-session.test.ts`

**Interfaces:**
- Produces:

```ts
export interface SessionHost {
  postToPage(msg: BridgeOutboundToPage): void;    // provider encodes via encodeOutbound → webview.postMessage
  /** VS Code 的 CustomDocumentContentChangeEvent 没有 dirty 布尔——每次 fire 都
   *  把文档标脏，只有 save/revert 能清。因此 host 只暴露「变脏了」这一个方向；
   *  session 内部维护 Rust 侧 dirty 布尔，仅在 false→true 边沿调用一次
   *  contentChanged()（true→false 不通知 VS Code——undo 回到干净点后标签仍显
   *  示脏是已接受的限制，保存/revert 即清）。 */
  contentChanged(): void;
  writeFile(bytes: Uint8Array): Promise<void>;     // atomic save to the doc uri
  writeBackup(name: string, bytes: Uint8Array): Promise<void>; // conflict/hot-exit backups; MUST throw on failure
  /** Durable fallback for the accept-remote obligation when writeBackup
   *  fails twice: e.g. a user save dialog. Only resolves once bytes are
   *  actually persisted somewhere the user can find. */
  writeBackupFallback(bytes: Uint8Array): Promise<string>; // returns the persisted location description
  showConflictDialog(serverVersion: number): Promise<"use-local" | "accept-remote" | undefined>;
  /** External-disk-change three-way prompt (distinct from the MCP conflict
   *  dialog): "reload" discards local; "keep-local" keeps the editor state;
   *  "save-disk-copy" durably writes the DISK bytes somewhere else BEFORE
   *  the local editor is kept (both versions survive). undefined = dismissed
   *  (treated as keep-local, re-promptable on the next change event). */
  showExternalChangeDialog(): Promise<"reload" | "keep-local" | "save-disk-copy" | undefined>;
  /** Timer abstraction for the init retry loop; returns a cancel fn. */
  schedule(fn: () => void, ms: number): () => void;
  warn(message: string): void;
}
export type SessionState = "booting" | "ready" | "open-pending" | "conflict" | "disposed";
export class PenSession {
  constructor(host: SessionHost, token: string, initialDocJson: string);
  /** Begins the init retry loop (host.schedule-driven): re-posts init every
   *  500ms (cap 20 tries → host.warn + state stays booting) until the page's
   *  `ready` arrives — webview.postMessage success does not prove receipt,
   *  and the Rust listener installs late in mount_ck. */
  start(): void;
  /** Feed every message from the webview relay. Unknown → ignored. */
  onPageMessage(raw: unknown): void;
  /** CustomDocument entry points — all reject if called before ready.
   *  Each accepts an optional isCancelled probe (VS Code CancellationToken
   *  adapted by the provider): checked when the request leaves the queue —
   *  already-cancelled entries reject with a Cancelled error. */
  save(isCancelled?: () => boolean): Promise<void>;        // snapshot(save) → writeFile → save-committed；对应 requestId 的 snapshot-conflict → reject(ConflictPending)
  backup(isCancelled?: () => boolean): Promise<Uint8Array>; // snapshot(backup) → bytes
  revert(diskJson: string): Promise<void>;   // open-document(diskJson) → wait opened
  externalFileChanged(diskJson: string, isRustDirty: boolean): Promise<void>;
  /** Rejects every queued/in-flight snapshot, open, and conflict waiter with
   *  a Disposed error, cancels the init retry timer, state=disposed; all
   *  later calls reject. */
  dispose(): void;
  get state(): SessionState;
  get isRustDirty(): boolean;
}
```

  **协议规则（全部来自 Plan 1 落地语义，测试逐条锁定）**：
  1. 构造后立即 `postToPage(init{token})`；收到 `ready` 前，save/backup/revert reject（"not ready"），随后发 `open-document{initialDocJson}`、等 `opened` 后 state=ready（宿主打开的字节是权威——daemon --file 加载的同一文件，opened 到达前 dirty 事件忽略）。
  2. `dirty-changed` → session 记录 Rust dirty 布尔；仅 false→true 边沿调 `host.contentChanged()`（state=ready 后才处理；语义见 SessionHost 注释）。`isRustDirty` 以 getter 暴露供 externalFileChanged/provider 查询。
  3. `save()`：发 `snapshot{purpose:"save",requestId}` → 等 `snapshot-result{requestId}` → `host.writeFile(docJson bytes)` **成功后** → 发 `save-committed{generation,revision}`（用 snapshot-result 里的对，不是当前值——写盘期间的新编辑不得误标已保存）。`snapshot-conflict` → 进入冲突流程（规则 5）后 save() reject（VS Code 显示保存失败，用户解决冲突后重试）。
  4. 并发纪律：同一时刻至多一个未决 snapshot（save/backup 排队串行）；requestId 单调递增字符串；不匹配的 `snapshot-result` 丢弃并 warn。
  5. 冲突（**单一冲突事务**）：session 同一时刻至多一个活动冲突事务。事务由首个冲突事件（`sync-conflict` 或某 snapshot 的 `snapshot-conflict`）开启；**事务活动期间到达的其它冲突事件只更新存储的 serverVersion、不再弹对话框**（Rust 的失败 snapshot 会既回 `snapshot-conflict` 又置闩发 `sync-conflict`——精确序列测试：snapshot-conflict 先到 + sync-conflict 后到 = 一次对话框）。事务开启 → `host.showConflictDialog(v)`：
     - `"use-local"` → 发 `resolve-conflict{mode:"use-local",requestId}` → 等 `conflict-resolved`；**完成条件与 accept-remote 同样分叉**：冲突发生于 open 流程（曾 open-pending）→ conflict-resolved 后仍须等 `opened` 才回 ready（Rust 先发 conflict-resolved、observer 后发 opened）；稳态 → conflict-resolved 即回 ready。两种场景测试；
     - `"accept-remote"` → 发 `resolve-conflict{mode:"accept-remote",requestId}` → **先等 `snapshot-result`（本地字节）并持久化成功，才允许会话继续**：`host.writeBackup` 失败 → 重试一次 → 仍失败 → `host.writeBackupFallback`（阻塞到用户把字节真正落盘为止）——期间到达的 `conflict-resolved` **缓冲不消费**，持久化完成后才消费。**完成条件按冲突场景分叉（Rust 落地语义：`opened` 只在存在挂起 open 时发射——sync_gate 的 open_pending；稳态 MCP 冲突的远端应用不发 opened，等它必死锁）**：
       - 冲突发生时会话正处 open 流程（state 曾为 open-pending）→ 等 `opened` 回 ready；
       - 稳态冲突（无挂起 open）→ 记录冲突时刻的 generation，等**第一条 generation 严格更大的 `dirty-changed`**（远端应用的 replace 必 bump generation、观察者按三元组变化必发）即回 ready；10s 超时（host.schedule）兜底回 ready + warn。两种场景都有测试。
       字节只存在内存 = 违反「两个版本都不许丢」，任何路径都不允许以此收场；
     - `"use-local"` 的等待也不是只有 conflict-resolved：Rust 重试失败会对**同一 requestId** 发 `snapshot-conflict{requestId, serverVersion}`（vscode_bridge 的 use-local 重推路径）——收到即保持 conflict 状态并以新 serverVersion **重弹对话框**（测试覆盖）；
     - 用户关闭对话框（undefined）→ 冲突事务保持开启（拉取仍被 Rust 侧门控挡着，无数据损坏）。**重入路径不是等下一条 `sync-conflict`（Rust 冲突闩是消费型的，门控持续冲突期间不会重发）**：conflict 状态下用户发起的 `save()`/`backup()` 不再直接 reject，而是**重弹对话框**（用存储的 serverVersion）——用户解决后该 save 继续或按解决结果 reject；测试：dismiss → save → 对话框重弹 → use-local 后 save 完成。
  6. `externalFileChanged(diskJson, isRustDirty)`：不 dirty → 直接 `revert(diskJson)`（自动重载）；dirty → `host.showExternalChangeDialog()`：`"reload"` → revert(diskJson)；`"keep-local"`/undefined → 不动；`"save-disk-copy"` → **先** `host.writeBackup("disk-copy-…", diskJson bytes)` 成功（失败走 writeBackupFallback，同 accept-remote 的持久化纪律）**再**保留本地。写失败与取消路径都有测试。
  7. `opened` 在 AcceptRemote 后由页面补发 → 若存在挂起 revert/open promise 则 resolve。
- Consumes: Task 2 协议类型。

- [ ] **Step 1: 写失败测试**（MockHost 记录调用序列 + 手动 timer；用例至少：start 后 init 重发直到 ready（含 ready 到达后 timer 取消断言）；init→ready→open→opened 启动序；ready 前 save reject；save 快乐路径断言 save-committed 携带 snapshot-result 的 (generation,revision)；写盘 reject 时**不发** save-committed；两个 save 排队串行；save 的 requestId 收到 snapshot-conflict → 该 save reject(ConflictPending)；已取消的排队项出队即 reject(Cancelled)；dispose 拒绝全部在途/排队 waiter 且后续调用全 reject；sync-conflict→use-local→resolved（稳态即回 ready；open 流程中须等 opened）；sync-conflict→accept-remote：**conflict-resolved 早到被缓冲**、backup 持久化完成先于消费、backup 两败后 writeBackupFallback 阻塞、之后等 opened 才回 ready；**单一冲突事务**：事务活动期间到达的第二个冲突事件（snapshot-conflict 先 + sync-conflict 后）**不重弹**、只更新 serverVersion；**dismiss 重入**：对话框取消 → 后续 `save()`（或 `backup()`）用存储 serverVersion **重弹**对话框 → use-local 后该 save 完成（**不测「等第二条 sync-conflict」——Rust 冲突闩消费型不会重发**）；externalFileChanged 三选各分支含 save-disk-copy 先写盘；乱序/未知 requestId 丢弃）
- [ ] **Step 2-5: 红 → 实现 → 绿 → Commit** — `feat(vscode): pen session protocol state machine`

---

### Task 6: webview 壳 + 消息中继（`src/vscode/webview-shell.ts`）

**Files:**
- Create: `src/vscode/webview-shell.ts`（生成 HTML 的纯函数 + CSP 常量）、`src/vscode/webview-shell.test.ts`

**Interfaces:**
- Produces: **两阶段 boot 协议**（解「spawn 需要 webview origin、iframe src 需要 daemon 端口」的先后依赖，并消灭 init 消息丢失竞态——`webview.postMessage` 成功≠对方收到，Rust 监听器在 `mount_ck` 里装得晚）：
  - 阶段 1 `buildBootHtml(nonce)`：无 iframe；脚本加载即 `acquireVsCodeApi().postMessage('{"type":"op-shell/ready","origin":"<window.origin>"}')`（**shell 真实 origin 从 `window.origin` 上报**——`asWebviewUri` 是资源 URI，不是文档 origin，不能用它推）。
  - 阶段 2（**唯一流程：整页替换，无 navigate 消息**）：扩展收到 shell-ready → 用上报 origin spawn daemon → `panel.webview.html = buildWebviewHtml({iframeSrc, nonce})`——完整第二版 HTML 自带 iframe 与完整 CSP（`default-src 'none'; frame-src <iframeOrigin>; script-src 'nonce-<nonce>'; style-src 'unsafe-inline'`）；重设 html 重建壳，第二次 shell-ready 由扩展幂等忽略。boot HTML 的 CSP 只含 script-src。满屏 iframe 不加 `sandbox`（同源 daemon 页面需要完整能力）。
  - 中继方向（VS Code 两条通道不对称，别搞反）：
    1. 扩展 → 壳：`webview.postMessage(jsonString)` → 壳 `window.addEventListener("message", e)`（`e.source` 非 iframe）→ 控制消息（`op-shell/*`）自行处理，其余转发 `iframe.contentWindow.postMessage(e.data, iframeOrigin)`（**显式目标 origin，绝不 `"*"`**）；
    2. 页面 → 壳：`e.source === iframe.contentWindow && e.origin === iframeOrigin`（**双条件**）→ `acquireVsCodeApi().postMessage(e.data)`（此 API 只用于 webview→扩展方向）；
    3. 载荷全程 JSON **字符串**（`typeof e.data === "string"` 才转发），壳不解析业务消息（只认 `op-shell/` 前缀控制消息）。
- Produces: 纯函数可单测：boot/full 两个 HTML 生成器；CSP、双条件校验、显式 origin 转发、无 `postMessage(…, "*")`（正则断言）、string 守卫、nonce、`op-shell/ready` 上报 `window.origin`。
- Consumes: 无（`asExternalUri` 的调用在 Task 7 的 provider 里）。

- [ ] **Step 1-5: 红 → 实现 → 绿 → Commit** — `feat(vscode): webview relay shell with strict csp and origin-pinned forwarding`

---

### Task 7: CustomEditorProvider + DaemonPool 接线（`src/vscode/pen-editor-provider.ts`、`src/daemon/daemon-pool.ts`）

**Files:**
- Create: `src/daemon/daemon-pool.ts`、`src/daemon/daemon-pool.test.ts`、`src/vscode/pen-editor-provider.ts`、`src/extension.ts`（注册 provider）

**Interfaces:**
- Produces（pool，无 vscode import）：

```ts
export class DaemonPool {
  constructor(spawn: (file: string, allowOrigin: string) => Promise<DaemonClient>, logger: DaemonLogger);
  async acquire(filePath: string, allowOrigin: string): Promise<DaemonClient>; // one daemon per file; concurrent acquires coalesce
  clientFor(filePath: string): DaemonClient | undefined;
  setActive(filePath: string | undefined): void; // McpProxy routing target; undefined 清空（回 -32002）
  get active(): { filePath: string; client: DaemonClient } | undefined;
  async release(filePath: string): Promise<void>; // await dispose + evict
  async disposeAll(): Promise<void>;              // awaits every client's dispose (deactivate chain)
  onActiveChanged(cb: () => void): void;
  /** Crash policy: a non-dispose exit respawns ONCE and then invokes this —
   *  the provider re-wires the session/webview to the NEW client (new port,
   *  new token: iframe src + init must be redone). A second crash notifies
   *  and evicts (cb receives client: undefined). */
  onRestart(cb: (filePath: string, client: DaemonClient | undefined) => void): void;
}
```

  另建 `src/session/session-registry.ts`（活动态的唯一来源是 provider 的 view-state 事件，不是 DaemonPool）：

```ts
export class SessionRegistry {
  register(filePath: string, session: PenSession): void;
  unregister(filePath: string): void;   // dispose 时调；若是 active 则清空 active
  /** undefined 清空活动态——用户切到非 OpenPencil 编辑器时 provider 必须
   *  clear，否则最后一个 .op 永远是 MCP/codegen 目标。 */
  setActive(filePath: string | undefined): void;
  activeSession(): PenSession | undefined;
}
```

  `DaemonPool.setActive` 同样接受 `string | undefined`（清空后 McpProxy 回到 -32002 无活动文档错误）。provider 责任：`resolveCustomEditor` 时若 `panel.active` 立即 `setActive`；`onDidChangeViewState`：变 active → 双 set；变 !active 且当前 active 是本文件 → 双 `setActive(undefined)`；webview dispose → `unregister`；重启重建成功 → 重新 `register/setActive`。测试跟随 pool 用例 + registry 独立用例（active 清空/切换/切到非 .op 面板）。
- Produces（provider，vscode 层薄壳）：`CustomEditorProvider<PenDocument>` 实现——
  - **`PenDocument` 类**（显式定义，`implements vscode.CustomDocument`）：`uri`、`bootJson: string`（openCustomDocument 填）、**`latestDurable: { source: "disk" | "save" | "revert" | "backup"; json: string }`**（重启恢复源——每次成功 save（快照字节）/revert（盘上字节）/backupCustomDocument（备份字节）后更新；session 干净时以现盘字节为准）、`session?: PenSession`（resolve 时填）、`dispose()`。
  - `openCustomDocument(uri, openContext)`: **不碰进程**（此时还没有 webview/origin）。读内容：`openContext.backupId` 存在 → 读备份文件字节（hot-exit 恢复路径），**`latestDurable = {source:"backup", json:备份字节}`**（恢复标记保持到下次 save/revert 覆盖为止——初始 Rust open 是 clean 的，若标成 "disk"，紧接的 daemon 崩溃重启会按「clean→读盘」把恢复内容丢掉）；否则读盘，`latestDurable = {source:"disk", json}`。返回 `new PenDocument(uri, json)`。
  - `resolveCustomEditor(document, panel)`（**两阶段 boot，顺序固定——origin 来自壳上报，不用 asWebviewUri 推**）：
    1. `panel.webview.options={enableScripts:true}`；**先**注册 `onDidReceiveMessage` 监听与 5s 超时 waiter，**再** `panel.webview.html = buildBootHtml(nonce)`（webview 脚本可能在 html 赋值后同步跑完——监听后装必丢 ready；测试覆盖「ready 在监听注册后立即同步送达」场景）；
    2. 等 `op-shell/ready{origin}` 控制消息（≤5s，超时错误页）；
    3. 探测二进制路径（设置 `openpencil.dev.daemonPath` → workspace `target/debug/op-host-web-server` → 报错引导页）→ `pool.acquire(uri.fsPath, shellOrigin)` + `DaemonHttp.ready()` 轮询（≤10s）；
    4. `asExternalUri(http://127.0.0.1:<port>/)` → `panel.webview.html = buildWebviewHtml({iframeSrc, nonce})`（完整版重建壳；随后的第二次 `op-shell/ready` 幂等忽略）；
    5. 构造 `PenSession(host, token, document.bootJson)` 存入 document.session + SessionRegistry → **`session.start()`**（init 重发循环由此开始，直到页面 `ready`）。
    SessionHost 映射：contentChanged→`_onDidChangeCustomDocument.fire({document})`（无 edits 数组——每次 fire 即标脏，VS Code 只在 save/revert 后清）、writeFile→先写 `<uri>.tmp` 再 `workspace.fs.rename(overwrite)`（原子写；写前置 `suppressWatcher` 标志；成功后更新 latestDurable）、writeBackup→`ExtensionContext.storageUri` 下、writeBackupFallback→`window.showSaveDialog` 循环、showConflictDialog/showExternalChangeDialog→`window.showWarningMessage({modal:true})`、schedule→`setTimeout` 包装、warn→通知。webview `onDidReceiveMessage` → 控制消息（`op-shell/*`）provider 自理，其余 → `session.onPageMessage`。
  - **daemon 崩溃重建**（`pool.onRestart(file, client)` 命中本文档）：旧 session `dispose()`；`client === undefined`（二次崩溃）→ 错误页；否则以**恢复源**重建（`pickRestartSource(document, isDirty)` 纯函数）：`isDirty` 为真 **或 `latestDurable.source === "backup"`**（hot-exit 恢复后尚未 save/revert——此时 Rust 侧 clean 但内容≠盘上，读盘即丢恢复内容）→ `latestDurable.json`；否则读现盘字节；重走两阶段 boot 3-5 步（新端口新 token）。**测试**：save 后重启用盘上字节、revert 后重启用 revert 字节、dirty+backup 后重启用备份字节、**hot-exit 恢复（clean）后立即崩溃重启用备份字节**。
  - `saveCustomDocument(document, cancellation)`/`saveCustomDocumentAs(document, dest, cancellation)`/`revertCustomDocument`/`backupCustomDocument(document, context, cancellation)` → session 对应方法；**backup 返回 `CustomDocumentBackup { id: <备份文件路径>, delete() }`**（字节写到 `context.destination`，成功后更新 `document.latestDurable = {source:"backup", json}`）；revert 成功后 `latestDurable = {source:"revert", json:盘上字节}`；saveAs：`session.backup()` 取字节写 dest（daemon 不换——文件身份归 VS Code）；cancellation token 适配为 `isCancelled` 探针传给 session 排队层（已取消 → reject）。
  - 文件 watcher：`createFileSystemWatcher(new vscode.RelativePattern(path.dirname(uri.fsPath), path.basename(uri.fsPath)))`（**API 要求 GlobPattern，不接受 Uri**）→ 事件先查 `suppressWatcher`（自己 tmp-write/rename 触发的事件带 300ms 消抖忽略）→ `session.externalFileChanged(读盘新字节, session.isRustDirty)`。
  - webview dispose → session dispose + registry 移除；文档 dispose → `pool.release`。
  - **注册**（extension.ts）：`window.registerCustomEditorProvider("openpencil.penEditor", provider, { supportsMultipleEditorsPerDocument: false, webviewOptions: { retainContextWhenHidden: true } })`——两项都是**注册选项**不是 manifest 属性；缺 retainContextWhenHidden 时隐藏面板被销毁、基于 snapshot 的 save/backup 会挂起。同文件二开由 `supportsMultipleEditorsPerDocument: false` 保证单面板。
- Consumes: Tasks 3-6 全部。

- [ ] **Step 1: pool 失败测试**（fake spawn 计数：同文件并发 acquire 只 spawn 一次；release 后再 acquire 重新 spawn；崩溃自动重启一次、二次崩溃不再重启；setActive/onActiveChanged 触发）
- [ ] **Step 2-4: 红 → 实现（pool 先行，provider 接线后靠 typecheck + Task 12 冒烟）→ 绿**
- [ ] **Step 5: Commit** — `feat(vscode): custom editor provider wired through daemon pool and pen session`

---

### Task 8: 外部变更三选 + 冲突 UI 细化（provider 层）

**Files:**
- Modify: `src/vscode/pen-editor-provider.ts`（外部变更提示、冲突对话框文案、备份路径策略）
- Modify: `src/session/pen-session.ts`（若 Task 5 的 externalFileChanged 钩子需要三选回调签名微调——保持可单测）
- Test: `src/session/pen-session.test.ts` 增补

**Interfaces:**
- Produces: 外部磁盘变更且编辑器 dirty 时三选 modal：「重载（丢弃我的修改）」「保留我的版本」「把磁盘版本另存为…」；不 dirty 自动重载（已有）。备份文件命名：`<basename>.conflict-<ISO时间戳>.op` 于文档同目录（写不进去 fallback 到 storageUri，通知给出完整路径）。冲突对话框文案含 serverVersion 与两个选项的后果说明。
- Consumes: Task 5/7。

- [ ] **Step 1-5: 红（session 三选路径测试）→ 实现 → 绿 → Commit** — `feat(vscode): external change and conflict resolution flows`

---

### Task 9: McpProxy 稳定端点（`src/mcp/mcp-proxy.ts`）

**Files:**
- Create: `src/mcp/mcp-proxy.ts`、`src/mcp/mcp-proxy.test.ts`

**Interfaces:**
- Produces（无 vscode import）：

```ts
export class McpProxy {
  constructor(pool: Pick<DaemonPool, "active" | "onActiveChanged">, logger: DaemonLogger);
  async listen(preferredPort: number): Promise<number>; // 0 = OS assigns; returns actual
  get port(): number;
  async dispose(): Promise<void>;
}
```

  行为：node `http.createServer` **显式 `listen(port, "127.0.0.1")`**；仅接受 `POST /mcp`（其余 404）；**校验 `Host` 头为 `127.0.0.1:<port>`/`localhost:<port>` 且若有 `Origin` 头必须拒绝**（浏览器跨站防线——MCP 客户端不发 Origin；发 Origin 的一律 403，防 DNS rebinding）；无活动文档 → 200 + JSON-RPC error `{code:-32002, message:"No active OpenPencil document. Open a .op file in the editor first."}`（取请求 id 回显）；有活动 → 转发到 `active.client` 的 `/mcp`（注入 `X-OpenPencil-Token`，透传 body 与 `mcp-session-id` 头，回传状态/头/体）；daemon 重启换端口对客户端透明（每请求现取 active）。
  **端口稳定性（主 AI 路线的生命线）**：激活时 `preferredPort` 取 `workspaceState.get("openpencil.proxyPort")`（设置项显式非 0 时优先设置项）；OS 分配后把实际端口 `workspaceState.update` 持久化——extension host 重启后复用同一端口，Task 10 写进 IDE 配置的 URL 才不会失效。首选端口被占 → 换新端口 + 持久化 + **主动提示重跑 configureMcp**（旧配置已陈旧）。listen 逻辑上层（extension.ts）负责 state 读写，McpProxy 本体保持无 vscode 依赖。
- Consumes: Task 7 pool。

- [ ] **Step 1: 失败测试**（stub 两个"daemon"HTTP 桩 + 可变 active：无活动 → -32002 且回显 id；活动 A → 转发到 A 且 token 头注入；切 active → 后续请求到 B；带 Origin 头 → 403；坏 Host → 403；daemon 桩 500 → 透传 500）
- [ ] **Step 2-5: 红 → 实现 → 绿 → Commit** — `feat(vscode): stable mcp proxy routing to the active document daemon`

---

### Task 10: McpConfigurator + IDE 适配表（`src/mcp/mcp-config.ts`）

**Files:**
- Create: `src/mcp/mcp-config.ts`（适配表 + JSONC 读改写，纯函数核心）、`src/mcp/mcp-config.test.ts`、`src/vscode/configure-command.ts`（命令壳：diff 确认 + 写入 + reload 提示）

**Interfaces:**
- Produces（纯函数层，输入现有文件文本 → 输出新文本，jsonc-parser 的 `modify`+`applyEdits` 保注释）：

```ts
export type IdeKind = "vscode" | "cursor" | "trae" | "windsurf";
export interface IdeProbe { appName: string; hasDir(rel: string): boolean } // feature probe: .cursor/.trae/.windsurf dir presence in workspace/home
export function detectIde(probe: IdeProbe): IdeKind; // appName 主判（含 "Cursor"/"Trae"/"Windsurf"），appName 不明确时用目录特征佐证（spec 要求 appName+特征双判）；均无 → vscode
export interface McpAdapter {
  kind: IdeKind;
  configPath(workspaceRoot: string): string;    // vscode: .vscode/mcp.json; cursor: .cursor/mcp.json; trae: .trae/mcp.json; windsurf: .windsurf/mcp.json
  upsert(existingText: string | null, proxyUrl: string): string; // server key "openpencil", url-type entry per each IDE's schema
  needsReload: boolean;
}
export function adapterFor(kind: IdeKind): McpAdapter;
```

  Schema 细节（写入形态，key 一律 `openpencil`）：vscode `{"servers":{"openpencil":{"type":"http","url":"<proxyUrl>"}}}`；cursor/trae/windsurf `{"mcpServers":{"openpencil":{"url":"<proxyUrl>"}}}`。已有同名条目 → 覆盖 url 字段保留其余；文件含注释/尾逗号（JSONC）→ 保留。**配置里只有 URL，绝无 token**（Plan 1 两级凭据契约）。
- Produces（命令壳）：`openpencil.configureMcp` —— 组 proxyUrl（`http://127.0.0.1:<proxy.port>/mcp`）→ 生成新文本 → 与现文本不同则 `window.showInformationMessage` 摘要 + 「查看 diff」（`vscode.diff` 虚拟文档）+「写入」确认 → 写入 → needsReload 则提示。首次激活（workspaceState 标记）自动弹一次建议。不受信 workspace → 命令直接提示受限。**错误路径**：多根 workspace → 取活动 .op 文档所在 folder，无活动文档则 QuickPick 选 folder；配置文件为只读/写失败 → 错误通知含路径；现文件 JSONC 解析失败（jsonc-parser 返回 parse errors）→ 不动原文件，提示手工处理。**逆操作**：`openpencil.removeMcp` 删除 `openpencil` 条目（同 diff 确认流；文件因此为空对象时保留文件不删）。
- Consumes: Task 9 proxy 端口。

- [ ] **Step 1: 失败测试**（每 IDE 一组 fixture：空文件、已有其它 server 的 JSONC（含注释）、已有 openpencil 旧 url——断言注释保留/其它 server 不动/url 更新；remove fixture：删除 openpencil 条目其余保留；malformed JSONC → 返回错误不产出文本；detectIde 全分支（appName 明确/不明确+目录佐证/双缺省）；输出文本无 "X-OpenPencil-Token" 字样）
- [ ] **Step 2-5: 红 → 实现 → 绿 → Commit** — `feat(vscode): mcp configurator with per-ide jsonc adapters`

---

### Task 11: Skill 安装 + Codegen + AiPanel 薄入口

**Files:**
- Create: `src/vscode/skill-command.ts`、`src/vscode/codegen-command.ts`、`src/vscode/ai-participant.ts`
- Modify: `src/extension.ts`（注册；`vscode.lm` 存在性守卫）
- Test: 可单测的部分（prompt 组装纯函数）`src/vscode/codegen-prompt.test.ts`

**Interfaces:**
- Produces:
  - `openpencil.installSkill`：skill 内容源 = **构建期从 `crates/op-cli/assets/skill-bundle.json` 抽取**（`openpencil-design` 只存在于该 bundle，**不在** `op skill:export` 的 op-ai-skills 注册表——不要走 skill:export）：Task 1 的 `build.mjs` 增一步 node 脚本读 JSON 抽出 markdown 写 `dist/assets/openpencil-skill.md`；命令把它写入当前 IDE 规则路径（cursor `.cursor/rules/openpencil.mdc`、trae `.trae/rules/openpencil.md`、windsurf `.windsurf/rules/openpencil.md`、vscode 提示走 MCP 即可无需规则文件）。写前确认。`openpencil.removeSkill` 删除对应文件（确认后）。
  - `openpencil.generateCode`：`vscode.lm` 可用 → 选模型 → 组 prompt（纯函数：经 **SessionRegistry.activeSession().backup()** 取 docJson + framework 设置；prompt 要求模型输出**结构化 JSON**：`{"files":[{"path":"relative/Component.tsx","content":"..."}]}`）→ 收齐流式输出后 `parseCodegenOutput(text)`（纯函数：提取 JSON（容忍 markdown 代码围栏包裹）、校验 files 数组、**拒绝**绝对路径/含 `..` 的路径/重复 path/单文件 >1MiB/总量 >10MiB，违规返回错误列表不写任何文件）→ 全部通过后写入用户选目录（存在文件冲突逐个确认）；解析失败 → 展示原始输出让用户自取。`vscode.lm` 不可用 → 打开 IDE 聊天引导消息（`env.clipboard` 写入预填提示词 + 通知说明）。**MVP 明确不做** codegen:plan/submit/assemble 多轮编排——单轮生成（spec 的 AI 驱动原则不变，编排属 post-MVP）。
  - `@openpencil` chat participant（manifest `chatParticipants` 条目 Task 1 已声明）：仅 `vscode.lm && vscode.chat` 存在时 `createChatParticipant`；handler：SessionRegistry 取活动会话 snapshot docJson 作上下文 + 用户 prompt → lm → 流式回复（只读顾问；写回画布 post-MVP——spec AiPanel 的 batch_design 写回属增强，MVP 声明降级并记录）。
- Consumes: Task 5（backup()）、**Task 7（SessionRegistry）**、**Task 9（无直接依赖但激活序在 proxy 之后）**。

**⚠️ 对 spec 的两处 MVP 收窄（写入 plan 即为决策记录，执行者不复议）**：codegen 单轮生成（不走 codegen:plan 协议）；AiPanel 只读不写回。理由：两者的完整形态依赖 vscode.lm 的稳定工具调用与更多 UX，收窄后仍满足「AI 主路线 = IDE Agent 经 MCP」（那条走 McpProxy 已完整）。

- [ ] **Step 1-5: 红（prompt 组装 + `parseCodegenOutput` 纯函数测试：合法输出、围栏包裹、绝对路径/`..`/重复/超限逐项拒绝、malformed 返回错误不产出）→ 实现 → 绿 → Commit** — `feat(vscode): skill install, single-shot codegen, and readonly chat participant`

---

### Task 12: 激活装配 + 受限模式 + 跨语言集成冒烟

**Files:**
- Modify: `src/extension.ts`（完整激活流：trust 检查 → pool/proxy/provider/命令注册 → deactivate 清理链）
- Create: `test/integration/daemon-contract.test.ts`（**真实二进制**冒烟）
- Modify: `packages/op-vscode/package.json`（`test:integration` script，依赖 `cargo build -p op-host-web-server`）

**Interfaces:**
- Produces: 激活流——**注册拓扑（一次性注册、后期绑定，避免重复注册抛错）**：
  - 命令：全部命令 ID **只注册一次**（无论信任状态），处理器统一经 `AppState` 间接调用——`AppState { trusted: boolean; assembled?: { pool, proxy, registry, … } }`；未授信/未装配时处理器弹受限提示（Task 10/11 的实现挂在 assembled 上，守卫是查 state 不是重注册）；
  - 编辑器 provider：`openpencil.penEditor` 视图类型**始终只有一个注册**——provider 实现内部按 `state.trusted` 分流：未授信 → resolve 出占位页（说明 + 信任按钮）并把 uri 记入 `placeholderUris`；已授信 → 走正常两阶段 boot。**没有第二个 provider 注册，也就没有 dispose/re-register 问题**；
  - `workspace.onDidGrantWorkspaceTrust`：幂等守卫下装配 `state.assembled`（pool/proxy/…）+ 置 `state.trusted = true` → 对 `placeholderUris` 逐个用 `vscode.window.tabGroups` 定位关闭 tab，再 `vscode.commands.executeCommand("vscode.openWith", uri, "openpencil.penEditor")` 重开（此时 provider 走已授信分支；纯 vscode 编排逻辑抽小函数 + 手动矩阵覆盖）；
  - trusted 启动 → 激活时直接装配；`deactivate` → `proxy.dispose()` + `pool.disposeAll()`（await stdin-EOF 链）。McpProxy 调用形态：`proxy.listen(persistedOrConfiguredPort)`（**参数只是端口**——`127.0.0.1` 绑定是 Task 9 McpProxy 内部职责）。
- Produces: 集成冒烟（node 侧，不进 vscode）：用真实 `target/debug/op-host-web-server` 走 DaemonClient.spawn → handshake → DaemonHttp.version → `McpProxy.listen(0)`（127.0.0.1 绑定是 proxy 内部行为）+ 经 proxy 发 MCP initialize（断言 result）→ 无 token 直连 daemon /api 401 → 带 Origin 头打 proxy 403 → dispose 链后进程退出。二进制缺失 → **测试失败并提示先 cargo build**（绝不 skip）。`ready()` 断言按 bundle 存在性分支：`crates/op-host-web/pkg` 存在 → 断言 true；不存在 → 断言 false 并 console 说明（wasm bundle 构建需 EMSDK，干净 checkout 不强求——**如实声明**：完整 open→ready→snapshot 字节比对属 wasm 页面行为，落在手动矩阵 Step 4 与 Plan 3 的浏览器 harness，不假装本冒烟覆盖）。
- Consumes: 全部前置任务。

- [ ] **Step 1: 写集成冒烟（红：extension.ts 装配缺失/接口未导出）**
- [ ] **Step 2: 实现激活装配 + 受限模式**
- [ ] **Step 3: `cargo build -p op-host-web-server && bun run test:integration` 绿；`bun test` 全绿；typecheck/lint 过**
- [ ] **Step 4: 手动矩阵首验（本机 VS Code：`code --extensionDevelopmentPath` 打开示例 .op → 编辑器渲染、改动 dirty、保存落盘、Cursor 里配置 MCP 后 AI 读到文档）——结果记录进 commit body；此为 spec 手动矩阵的第一行，其余行随 Plan 3 分发后补**
- [ ] **Step 5: Commit** — `feat(vscode): activation assembly, restricted mode, and cross-language smoke`

---

## 任务依赖图

```
Task 1 ─► Task 2 ─► Task 5 ─┐
       └► Task 3 ─► Task 4 ─┼─► Task 7 ─► Task 8 ──────────┐
       └► Task 6 ───────────┘   │                          │
                                └► Task 9 ─► Task 10 ──────┼─► Task 12
Task 5 + Task 7 + Task 9 ─► Task 11 ───────────────────────┘
```

## 覆盖对照（spec 组件 → 任务）

| spec 组件 | 任务 |
|---|---|
| DaemonPool（每文档进程、握手、崩溃重启一次、日志通道） | 3, 7 |
| CustomEditorProvider（dirty/save/backup/revert/外部变更/同文件二开/受限模式） | 5, 6, 7, 8, 12 |
| 冲突二选一 + accept-remote 备份义务 | 5, 8 |
| McpProxy（稳定端点、无活动文档结构化错误、Origin/Host 防线、无 secret 落盘） | 9, 10 |
| McpConfigurator（4 IDE 适配表、JSONC、diff 确认、reload 标记） | 10 |
| Install AI Skill | 11 |
| Codegen（AI 驱动；MVP 收窄单轮） | 11 |
| AiPanel（lm 守卫；MVP 收窄只读） | 11 |
| 不受信 workspace 受限模式 | 12 |
| 错误处理表（握手超时/崩溃/端口/写盘失败/配置冲突） | 3, 5, 7, 9, 10 |

## Plan 3 预告（不在本 plan 内）

平台 vsix 矩阵（binaries + web-bundle/CanvasKit 资产装配）、Marketplace/OpenVSX/Trae 发布、CI、`openpencil.dev.daemonPath` 让位于内置二进制解析、签名/公证。
