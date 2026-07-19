# HTML 导入子项目 2(全平台入口接线)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把已落地的 `op-html` 核心转换器接到所有人工/URL 入口:桌面与 web 的粘贴、拖拽/打开 `.html` 文件、URL 静态抓取(服务端,带 SSRF 防护),并补上外链资源(CSS/图片)抓取与内嵌。

**Architecture:** op-html 增加 `ResourceFetcher` 回调(crate 本体仍不碰网络,wasm32-clean);桌面入口镜像 Figma 链路(粘贴 worker 线程 + `paste_figma_nodes`,文件走后台 import session + `install_imported_state`);web 入口在 wasm 内同步解析(无 fetcher,外链资源告警降级——wasm 无法同步 fetch,这是 spec 认可的降级);URL 抓取实现为 op-host-services 里的新 MCP 工具 `import_html_url`(reqwest + `provider_dial::client_for(PublicOnly)` SSRF 防护 + `read_capped` 流式限量),CLI 检测 URL 前缀路由到它。

**Tech Stack:** 既有 `op-html`/`op-figma` 模式;`reqwest 0.12`(仅 native crate,严禁进 op-html/op-host-web);`provider_dial.rs`/`web_credentials.rs` 的 SSRF 组件;`web_image_search.rs` 的 `read_capped`/`ImageJobSlot` 限量模式。

**Spec:** `docs/superpowers/specs/2026-07-17-html-import-design.md` 子项目 2 节。

## Global Constraints

- 单文件 ≤ **800 行**;`.rs` snake_case;源码注释英文;Conventional Commits(scope:`html`/`desktop`/`web`/`mcp`/`cli`)。
- **op-html 保持 wasm32-clean**:新增依赖为零(fetcher 是 `dyn Fn` 回调);每个 op-html 任务后跑 `cargo check --target wasm32-unknown-unknown -p op-html`。
- **reqwest 不进 workspace.dependencies**(根 Cargo.toml:47 明文黑名单),按 crate 声明;op-cli **不得**新增 reqwest(它没有 HTTP 能力,URL 一律转发服务端)。
- 防御上限对齐仓库现值:单资源 ≤ **4 MiB**(`MAX_EMBEDDED_IMAGE_BYTES` 同值);资源总数 ≤ **200**;并发抓取槽复用 `ImageJobSlot` 模式(上限 4);所有网络读用 `read_capped` 流式截断;抓取超时 20s(对齐 `remote_image_host.rs:127`)。
- **SSRF**:`import_html_url` 一律 `EndpointDialPolicy::PublicOnly` + `OPENPENCIL_WEB_AI_ENDPOINT_ALLOWLIST` 允许表(本地 dev server 导入需操作者显式允许 —— 安全默认优先,因为该工具经 `/api/mcp` 对浏览器暴露)。
- 警告面向用户的呈现遵循现状:桌面 `eprintln!("[import-html] warning: …")`、web `console_warn`(与 figma 完全一致);**不新增 UI 文案,故本计划无 i18n 任务**。
- 每任务提交:`cargo fmt -p <crates>` 后 `git commit --no-verify`(本机 stable/nightly rustfmt 差异会让钩子在 legacy 文件误报);只 `git add` 各任务 Files 列出的确切路径;工作树里他人未提交改动绝不触碰。
- 执行者若在沙箱中无法运行 git:跳过提交步骤,由主会话按任务边界补提交(与子项目 1 相同约定)。

## 关键既有接口(全计划引用,实现前先读原文)

- `op_html::import_html(source, &HtmlImportOptions) -> HtmlImportResult{nodes, warnings}`(op-html/src/lib.rs:41)
- 桌面粘贴链:`keyboard_input.rs:423 handle_paste_payload`(HTML 分支在 :444,`payload.html: Option<String>`)、`:499 try_figma_clipboard_paste`、`:518 pump_figma_clipboard_paste`、通道字段 `main.rs:203 pending_figma_paste`、pump 调用点 `app_handler.rs:639`
- `paste_figma_nodes(nodes, viewport_w, viewport_h) -> bool`:native `widget_host.rs:1306`、web `file_ingest.rs:50`(内容无关,可直接复用)
- 桌面文件:`doc_io.rs:324 is_supported_figma_import`、app_handler.rs 三分支(289/483/1190)、`figma_import_session.rs` 全形(spawn/pump/cancel/PreparedImport/PumpOutcome)、`widget_host.rs:1221 install_imported_state`
- web:`dom_io.rs:812 handle_paste_event`(figma 分支 :820)、`:888 route_dropped_file`、`file_actions.rs:322 DropKind`/`:335 drop_kind`/`:281 ingest_figma_bytes`、`file_ingest.rs:26 install_ingested_state`、`read_file(file, ReadMode::Text, cb)`(dom_io.rs ~:1000)
- SSRF:`provider_dial.rs:16 EndpointDialPolicy`/`:39 client_for`/`:86 screen_resolved_addrs`;`web_credentials.rs:122 validate_web_provider_base_url_with_allowlist`/`:209 is_restricted_ip`
- 限量:`web_image_search.rs:26 MAX_IN_FLIGHT_IMAGE_JOBS`/`:32 ImageJobSlot`/`:582 read_capped`/`:558 fetch_image_data_url`;阻塞包装先例 `remote_image_host.rs:120 fetch_remote_image_blocking`(单线程 tokio `block_on`)
- HTTP client 构造:`chat_builtin_http.rs:567 builtin_http_client_builder()`(带 connect/request 超时)
- MCP 工具注册:`mcp_serve.rs:589 register_tool!` 区、`mcp_serve/schemas.rs:122` 一带 schema 数组、**精确计数断言** `mcp_serve/tests.rs:21`(当前 121,加一个工具必须 +1)

---

### Task 1: op-html — ResourceFetcher 与外链样式表

**Files:**
- Create: `crates/op-html/src/resources.rs`
- Modify: `crates/op-html/src/lib.rs`(声明 `pub mod resources;`,新增 `import_html_with_resources`)

**Interfaces:**
- Produces:
  - `pub type ResourceFetcher<'a> = dyn Fn(&str) -> Option<Vec<u8>> + 'a;`(签名与 `op_figma::ImageTransform` 同风格;**不**依赖 op-figma——那会拖进 kiwi 解析器,此处本地定义形状兼容的别名)
  - `pub type ImageTransform<'a> = dyn Fn(&[u8]) -> Option<Vec<u8>> + 'a;`(同上,供 Task 2)
  - `resources.rs`:`pub fn resolve_url(base: Option<&str>, href: &str) -> Option<String>` — 绝对 `http(s)://`/`data:` 原样;`//host/x` 补 base 的 scheme;相对路径按 RFC 3986 基本合并(路径段 `../`/`./` 规约;无 base 时返回 None)
  - `HtmlImportOptions` 增加字段:`pub base_url: Option<String>`(`Default` 为 None;serde 无关,纯构造)
  - `pub fn import_html_with_resources(source: &str, opts: &HtmlImportOptions, fetcher: Option<&ResourceFetcher>, transform: Option<&ImageTransform>) -> HtmlImportResult` — 本任务先接**外链 CSS**:DOM 收集阶段额外收集 `<link rel="stylesheet" href=...>`(dom.rs 已丢弃 link 标签——改为把 `(rel, href)` 收进 `ParsedDom.stylesheet_links: Vec<String>`);对每个 link:`resolve_url` → fetcher 取回 → UTF-8 化(lossy)→ 作为作者样式表参与级联(order 排在 `<style>` 块之前,与文档顺序一致的近似);fetcher 为 None 或取回失败 → warning `external stylesheet skipped: <url>`
  - 资源计数器:`resources.rs` 内 `pub(crate) struct ResourceBudget { count: usize }`,上限 **200**(CSS+图片共享),超限 → warning 一次 + 后续跳过
  - 现有 `import_html(source, opts)` 变为 `import_html_with_resources(source, opts, None, None)` 的薄包装(行为不变,既有 42 个测试必须原样通过)

- [ ] **Step 1: 写失败测试(resources.rs 底部 + e2e_tests.rs 追加)**

```rust
// resources.rs tests
#[test]
fn resolve_url_forms() {
    assert_eq!(resolve_url(Some("https://a.dev/x/y.html"), "s.css").as_deref(),
               Some("https://a.dev/x/s.css"));
    assert_eq!(resolve_url(Some("https://a.dev/x/y.html"), "/s.css").as_deref(),
               Some("https://a.dev/s.css"));
    assert_eq!(resolve_url(Some("https://a.dev/x/y.html"), "../s.css").as_deref(),
               Some("https://a.dev/s.css"));
    assert_eq!(resolve_url(Some("https://a.dev/x/"), "//cdn.b.io/s.css").as_deref(),
               Some("https://cdn.b.io/s.css"));
    assert_eq!(resolve_url(None, "https://c.io/s.css").as_deref(), Some("https://c.io/s.css"));
    assert!(resolve_url(None, "s.css").is_none());
}

// e2e_tests.rs 追加
#[test]
fn external_stylesheet_participates_in_cascade() {
    let html = r#"<html><head><link rel="stylesheet" href="site.css"></head>
        <body><p class="hot">x</p></body></html>"#;
    let fetcher = |url: &str| -> Option<Vec<u8>> {
        (url == "https://a.dev/site.css").then(|| b".hot { color: #ff0000 }".to_vec())
    };
    let opts = HtmlImportOptions { base_url: Some("https://a.dev/page.html".into()), ..Default::default() };
    let r = import_html_with_resources(html, &opts, Some(&fetcher), None);
    let PenNode::Frame(root) = &r.nodes[0] else { panic!() };
    let PenNode::Frame(p) = &root.children.as_ref().unwrap()[0] else { panic!() };
    let PenNode::Text(t) = &p.children.as_ref().unwrap()[0] else { panic!() };
    let Some(fills) = &t.fill else { panic!("text should carry color fill") };
    assert!(matches!(&fills[0], jian_ops_schema::style::PenFill::Solid(s) if s.color == "#ff0000"));
}

#[test]
fn missing_fetcher_degrades_with_warning() {
    let html = r#"<link rel="stylesheet" href="https://a.dev/s.css"><p>x</p>"#;
    let r = import_html(html, &HtmlImportOptions::default());
    assert!(r.warnings.iter().any(|w| w.contains("external stylesheet skipped")));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html resources`、`cargo test -p op-html e2e`。
- [ ] **Step 3: 实现**(dom.rs 收集 `<link>`:仅 `rel` 含 `stylesheet` 的收 href,其余 link 仍丢弃;lib.rs 编排在 UA/`<style>` 之外插入外链表的 `parse_stylesheet(css, 500 + i*10_000)`——order 介于 UA(0) 与 `<style>`(1000+) 之间偏保守即可,精确文档序不追求)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html`(全量,含既有 42 个)+ wasm check。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): resource fetcher plumbing and external stylesheet cascade"
```

---

### Task 2: op-html — 图片内嵌与整文档包装

**Files:**
- Modify: `crates/op-html/src/resources.rs`(图片内嵌走查)
- Modify: `crates/op-html/src/lib.rs`(编排 + `import_html_document`)

**Interfaces:**
- Consumes: Task 1 的 fetcher/transform/ResourceBudget/resolve_url
- Produces:
  - `resources.rs`:`pub(crate) fn embed_images(nodes: &mut [PenNode], base_url: Option<&str>, fetcher: &ResourceFetcher, transform: Option<&ImageTransform>, budget: &mut ResourceBudget, warnings: &mut Vec<String>) -> usize` — 递归走查:`PenNode::Image` 的 `src` 与容器 `fill` 里 `PenFill::Image` 的 `url`,凡非 `data:` 开头的 → `resolve_url` → fetcher → transform(Some 则替换字节)→ `data:{mime};base64,` 内嵌(MIME 魔数嗅探:`FF D8`→jpeg、`47 49`→gif、`52 49`→webp、`3C`/`3c 73 76 67`→svg+xml、默认 png——与 `op-figma/src/image_resolver.rs:25 blob_to_data_url` 同规则,本地实现,base64 依赖已有);取回失败 → 该节点降级为灰底占位(Image src 换成 1×1 灰 PNG data URL 常量 `PLACEHOLDER_GRAY_PNG`)+ warning
  - 同一 URL 去重:走查内 `HashMap<String, String>`(url→data url)缓存,重复引用不重复计数
  - `pub fn import_html_document(source: &str, opts: &HtmlImportOptions, fetcher: Option<&ResourceFetcher>, transform: Option<&ImageTransform>) -> HtmlDocumentResult`;`pub struct HtmlDocumentResult { pub document: jian_ops_schema::document::PenDocument, pub warnings: Vec<String> }` — 把 `import_html_with_resources` 的根 Frame 包成 `PenDocument { version: "1.0".into(), name: <root frame name>, children: nodes, 其余字段 None/默认 }`(**实现前读 `vendor/jian/crates/jian-ops-schema/src/document.rs` 确认字段全集**,缺省字段逐一列 None;若有 `Default` 则 `..Default::default()`)。供"打开 .html 文件 → 整文档"路径使用

- [ ] **Step 1: 写失败测试(e2e_tests.rs 追加)**

```rust
#[test]
fn images_embed_via_fetcher_with_dedup_and_placeholder() {
    let html = r#"<div><img src="a.png"><img src="a.png"><img src="missing.png"></div>"#;
    let png: Vec<u8> = vec![0x89, 0x50, 0x4e, 0x47, 1, 2, 3];
    let fetched = std::cell::RefCell::new(0usize);
    let fetcher = |url: &str| -> Option<Vec<u8>> {
        *fetched.borrow_mut() += 1;
        (url == "https://a.dev/a.png").then(|| png.clone())
    };
    let opts = HtmlImportOptions { base_url: Some("https://a.dev/p.html".into()), ..Default::default() };
    let r = import_html_with_resources(html, &opts, Some(&fetcher), None);
    let PenNode::Frame(root) = &r.nodes[0] else { panic!() };
    let PenNode::Frame(div) = &root.children.as_ref().unwrap()[0] else { panic!() };
    let kids = div.children.as_ref().unwrap();
    let PenNode::Image(i1) = &kids[0] else { panic!() };
    let PenNode::Image(i2) = &kids[1] else { panic!() };
    let PenNode::Image(i3) = &kids[2] else { panic!() };
    assert!(i1.src.as_str().starts_with("data:image/png;base64,"));
    assert_eq!(i1.src.as_str(), i2.src.as_str()); // dedup: same data url
    assert_eq!(*fetched.borrow(), 2); // a.png fetched once + missing.png once
    assert!(i3.src.as_str().starts_with("data:image/png;base64,")); // gray placeholder
    assert!(r.warnings.iter().any(|w| w.contains("missing.png")));
}

#[test]
fn transform_callback_rewrites_bytes() {
    let html = r#"<img src="https://a.dev/big.jpg">"#;
    let fetcher = |_: &str| Some(vec![0xffu8, 0xd8, 9, 9, 9, 9]);
    let transform = |_: &[u8]| Some(vec![0xffu8, 0xd8, 1]); // "downscaled"
    let r = import_html_with_resources(html, &HtmlImportOptions::default(),
        Some(&fetcher), Some(&transform));
    let PenNode::Frame(root) = &r.nodes[0] else { panic!() };
    let PenNode::Image(img) = &root.children.as_ref().unwrap()[0] else { panic!() };
    use base64::Engine as _;
    let b64 = img.src.as_str().strip_prefix("data:image/jpeg;base64,").unwrap();
    assert_eq!(base64::engine::general_purpose::STANDARD.decode(b64).unwrap(), vec![0xff, 0xd8, 1]);
}

#[test]
fn document_wrapper_produces_pendocument() {
    let r = import_html_document("<html><head><title>T</title></head><body><p>x</p></body></html>",
        &HtmlImportOptions::default(), None, None);
    assert_eq!(r.document.children.len(), 1);
    assert_eq!(r.document.name.as_deref(), Some("T"));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html e2e`。
- [ ] **Step 3: 实现**(注意闭包捕获 RefCell 的测试要求 fetcher 参数是 `&dyn Fn` 而非 `fn`——类型别名已是 dyn;background `PenFill::Image` 的 `url` 字段类型 `ImageSrc`,用 `ImageSrc::from(String)` 赋回)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html` + wasm check + `cargo clippy -p op-html --all-targets -- -D warnings`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): image embedding via fetcher and whole-document wrapper"
```

---

### Task 3: 桌面粘贴 HTML

**Files:**
- Modify: `crates/op-host-desktop/src/keyboard_input.rs`(HTML 分支 + try/pump 函数)
- Modify: `crates/op-host-desktop/src/main.rs`(`pending_html_paste` 字段,伴随 `pending_figma_paste` 声明:203/初始化:412)
- Modify: `crates/op-host-desktop/src/app_handler.rs`(pump 调用点 :639 旁 + redraw 门 :872/:1491 旁)
- Modify: `crates/op-host-desktop/Cargo.toml`(加 `op-html = { path = "../op-html" }`)

**Interfaces:**
- Consumes: `op_html::import_html`(粘贴路径无 fetcher——剪贴板 HTML 无可靠 base URL,外链资源告警降级,行为与 spec"拿不到就告警降级"一致);`host.paste_figma_nodes`
- Produces:
  - `keyboard_input.rs`:`fn try_html_clipboard_paste(&mut self, html: String) -> Option<bool>`、`pub(crate) fn pump_html_clipboard_paste(&mut self) -> bool`
  - `main.rs`:`pending_html_paste: Option<std::sync::mpsc::Receiver<(Vec<jian_ops_schema::node::PenNode>, Vec<String>)>>`
- 行为:`handle_paste_payload` 的 HTML 分支(keyboard_input.rs:444)改为:

```rust
if let Some(html) = payload.html.take() {
    if let Some(result) = self.try_figma_clipboard_paste(html.clone()) {
        return result;
    }
    if let Some(result) = self.try_html_clipboard_paste(html) {
        return result;
    }
}
```

`try_html_clipboard_paste`:`html.trim().is_empty()` → None(落回后续 text/image 分支);否则 worker 线程 `op_html::import_html(&html, &HtmlImportOptions::default())`,`tx.send((r.nodes, r.warnings))`,置 `pending_html_paste`,返回 `Some(true)`。`pump_html_clipboard_paste`:`try_recv` 到 `(nodes, warnings)` 后逐条 `eprintln!("[import-html] warning: {w}")`;`nodes` 为空 → 只清通道返回 false(粘贴静默放弃,与空 figma 剪贴板一致);非空 → 根 Frame 是"整页 body"包装,粘贴场景取其 children 更自然?**不**——保持整根插入(与 MCP 工具行为一致,用户得到一个命名 Frame,可编辑性更好),直接 `self.host.paste_figma_nodes(nodes, self.viewport_width, self.viewport_height)`。

- [ ] **Step 1: 写失败测试**

桌面 host 无 headless 测试先例(键盘链路依赖 winit),本任务的可测单元是决策函数。在 `keyboard_input.rs` 底部现有 tests 模块(若无则新建 `#[cfg(test)] mod html_paste_tests`)加纯函数测试:抽出 `fn html_paste_should_consume(html: &str) -> bool`(供 try_html_clipboard_paste 调用的守卫:非空白即 true)并测:

```rust
#[test]
fn html_paste_guard() {
    assert!(!html_paste_should_consume("   \n"));
    assert!(html_paste_should_consume("<div>x</div>"));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-host-desktop html_paste`(编译失败:函数不存在)。
- [ ] **Step 3: 实现全部接线**(main.rs 字段、app_handler pump:`if self.pump_html_clipboard_paste() { self.request_redraw(true); }` 与 figma pump 同位;redraw 门两处 `|| self.pending_html_paste.is_some()` 仿 figma 写法——**先读 :872/:1491 原文照抄形状**)。
- [ ] **Step 4: 验证** — `cargo test -p op-host-desktop html_paste` 通过;`cargo check -p op-host-desktop` 通过;`cargo build -p op-host-desktop` 通过。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-host-desktop
git add crates/op-host-desktop/src/keyboard_input.rs crates/op-host-desktop/src/main.rs crates/op-host-desktop/src/app_handler.rs crates/op-host-desktop/Cargo.toml
git commit --no-verify -m "feat(desktop): paste html clipboard as editable nodes"
```

---

### Task 4: 桌面打开/拖拽 .html 文件(后台 import session + 本地资源)

**Files:**
- Modify: `crates/op-host-services/src/doc_io.rs`(`is_supported_html_import`)
- Create: `crates/op-host-desktop/src/html_import_session.rs`(镜像 `figma_import_session.rs`)
- Modify: `crates/op-host-desktop/src/app_handler.rs`(三个路由分支 :289/:483/:1190 旁 + pump :642 旁)
- Modify: `crates/op-host-desktop/src/main.rs`(`current_html_import` 字段 :200 旁;文件过滤 :586-587/:648-649/:959-960;drain :614/:659)

**Interfaces:**
- Consumes: Task 2 `op_html::import_html_document`;`figma_import_session.rs` 的 PreparedImport/PumpOutcome 形状;`widget_host.rs:1221 install_imported_state`;`image_downscale::maybe_downscale`
- Produces:
  - `doc_io.rs`:`pub fn is_supported_html_import(path: &std::path::Path) -> bool`(大小写不敏感 `html`/`htm`,镜像 :324)
  - `html_import_session.rs`:`pub struct HtmlImportSession`、`pub fn spawn(host, path) -> HtmlImportSession`、`pub fn pump(host, session, current_path, window) -> PumpOutcome`、`pub fn cancel(host, session)` — 结构逐项镜像 figma 版(含 `figma_import_in_progress` 旗标复用:**读 figma 版确认旗标名后沿用同一旗标**,避免新增 UI 状态;PumpOutcome 枚举直接 `use figma_import_session::PumpOutcome`)
  - 后台线程 `parse_path`:读文件文本 → **本地 fetcher**:相对路径按文件所在目录 `std::fs::read`(拒绝越出:`resolve` 后 `canonicalize` 前缀必须在文件父目录内,越出 → None + 由 crate 记 warning);`http(s)://` 开头 → None(桌面文件导入不做网络,URL 属 Task 6 的服务端工具;warning 引导)→ `import_html_document(源, opts{base_url: None, document_name: Some(文件名 stem)}, Some(&fetcher), Some(&transform))`,transform 用 `|b| image_downscale::maybe_downscale(b).map(|(_m, out)| out)`(镜像 figma_import_session.rs:97)→ `EditorState::from_document(r.document)`
- app_handler 三分支形状(每处):

```rust
} else if op_host_services::doc_io::is_supported_html_import(&path) {
    html_import_session::cancel(&mut self.host, &mut self.current_html_import);
    self.current_html_import = Some(html_import_session::spawn(&mut self.host, path));
    self.request_redraw(true);
}
```

- [ ] **Step 1: 写失败测试**

```rust
// doc_io.rs 现有 tests 模块追加
#[test]
fn html_import_extensions() {
    use std::path::Path;
    assert!(is_supported_html_import(Path::new("a.html")));
    assert!(is_supported_html_import(Path::new("A.HTM")));
    assert!(!is_supported_html_import(Path::new("a.svg")));
}
```

`html_import_session.rs` 的可测单元:本地 fetcher 的路径防越界。抽 `fn local_resource_fetch(dir: &std::path::Path, href: &str) -> Option<Vec<u8>>` 并测(用 tempdir?桌面 crate 无 tempfile 依赖则用 `std::env::temp_dir()` 下手工建目录):

```rust
#[test]
fn local_fetch_confines_to_directory() {
    let dir = std::env::temp_dir().join("op_html_fetch_test");
    let _ = std::fs::create_dir_all(dir.join("sub"));
    std::fs::write(dir.join("a.css"), b"x").unwrap();
    assert_eq!(local_resource_fetch(&dir, "a.css").as_deref(), Some(b"x".as_ref()));
    assert!(local_resource_fetch(&dir, "../outside.css").is_none());
    assert!(local_resource_fetch(&dir, "/etc/hosts").is_none());
    assert!(local_resource_fetch(&dir, "https://a.dev/x.css").is_none());
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-host-services doc_io`、`cargo test -p op-host-desktop html_import`。
- [ ] **Step 3: 实现**(main.rs 文件过滤三处把 `html`/`htm` 加进允许扩展名列表——**先 grep 三处原文确认列表形式**;drain 两处沿用 figma 分支形状)。
- [ ] **Step 4: 验证** — 两个 crate 测试通过 + `cargo build -p op-host-desktop`;**真机冒烟**:`echo '<h1 style="color:#357">Hi</h1>' > /tmp/t.html && cargo run -p op-host-desktop -- /tmp/t.html`(启动后应看到标题 Frame;无显示环境则跳过并记录)。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-host-desktop -p op-host-services
git add crates/op-host-services/src/doc_io.rs crates/op-host-desktop/src/html_import_session.rs crates/op-host-desktop/src/app_handler.rs crates/op-host-desktop/src/main.rs
git commit --no-verify -m "feat(desktop): open and drop .html files via background import session"
```

---

### Task 5: web 粘贴 + 拖拽 .html

**Files:**
- Modify: `crates/op-host-web/src/dom_io.rs`(粘贴分支 :820 后、`route_dropped_file` :888 加臂)
- Modify: `crates/op-host-web/src/file_actions.rs`(`DropKind::Html` :322、`drop_kind` :335、`ingest_html_source`)
- Modify: `crates/op-host-web/Cargo.toml`(加 `op-html = { path = "../op-html" }`)

**Interfaces:**
- Consumes: `op_html::{import_html, import_html_document, HtmlImportOptions}`(wasm 内同步解析,fetcher **必须**传 None——wasm 无同步 IO;外链资源自动降级为 warnings);`paste_figma_nodes`(file_ingest.rs:50)、`install_ingested_state`(file_ingest.rs:26)、`read_file(file, ReadMode::Text, cb)`
- Produces:
  - `file_actions.rs`:`DropKind` 增 `Html` 变体;`drop_kind`:`html`/`htm` → `Html`;`pub fn ingest_html_source(source: &str, file_name: &str) -> Result<IngestedDoc, String>` — `import_html_document(source, &opts{document_name: Some(file_name)}, None, None)` → `IngestedDoc { state: EditorState::from_document(r.document), warnings: r.warnings }`(**读 :281 ingest_figma_bytes 原文镜像其 IngestedDoc 构造与 preserve_authored_geometry 处理**;HTML 是 auto-layout 产物,`preserve_authored_geometry` 不设)
  - `dom_io.rs` 粘贴分支(figma 分支返回后、files 分支前):

```rust
if !html.is_empty() {
    evt.prevent_default();
    let result = op_html::import_html(&html, &op_html::HtmlImportOptions::default());
    for w in &result.warnings {
        console_warn(&format!("[import-html] {w}"));
    }
    if !result.nodes.is_empty() {
        let mut b = inner.borrow_mut();
        let (w, h) = b.viewport_size();
        if b.host_mut().paste_figma_nodes(result.nodes, w, h) {
            let _ = b.repaint();
        }
    }
    return;
}
```

(**注意**:此分支吞掉一切非 Figma 的 `text/html` 粘贴。浏览器复制纯文本时通常无 text/html;但富文本编辑器复制会带 html——这正是目标场景。`console_warn` 名字以 dom_io.rs 现有 helper 为准,grep 确认。)
  - `route_dropped_file` 加臂:`DropKind::Html => read_file(&file, ReadMode::Text, Box::new(move |v| { /* js string → ingest_html_source → install_ingested_state + fit_content_to_viewport, 镜像 ingest_figma_file 回调形状 */ }))`

- [ ] **Step 1: 写失败测试(file_actions.rs 现有 tests 模块追加;wasm 无关的纯函数可在 native 测)**

```rust
#[test]
fn drop_kind_recognizes_html() {
    assert!(matches!(drop_kind("page.html"), DropKind::Html));
    assert!(matches!(drop_kind("PAGE.HTM"), DropKind::Html));
    assert!(matches!(drop_kind("a.svg"), DropKind::Svg));
}

#[test]
fn ingest_html_source_builds_state() {
    let r = ingest_html_source("<h1>T</h1>", "page");
    assert!(r.is_ok(), "{r:?}");
}
```

(结构性断言(state 内有 1 个顶层 Frame)按 `ingest_figma_bytes` 既有测试的 EditorState 访问方式追加一条——**先看 file_actions.rs 既有测试怎么断言 IngestedDoc,照抄其访问路径**;上面的 Ok 断言是底线。)

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-host-web file_actions`(注:op-host-web 原生测试若需 feature 门,先 grep 现有测试的运行方式,如 `cargo test -p op-host-web --no-default-features --features web` 不可行时以 crate 现状测试命令为准)。
- [ ] **Step 3: 实现**,含 dom_io.rs 两处。
- [ ] **Step 4: 验证** — file_actions 测试通过;**wasm 编译门**:`cargo check --target wasm32-unknown-unknown -p op-host-web --no-default-features --features web` 必须通过(op-html 首次进 wasm 依赖图的完整验证)。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-host-web
git add crates/op-host-web/src/dom_io.rs crates/op-host-web/src/file_actions.rs crates/op-host-web/Cargo.toml
git commit --no-verify -m "feat(web): paste and drop html into the editor"
```

---

### Task 6: `import_html_url` MCP 工具(服务端 URL 抓取 + SSRF 防护)

**Files:**
- Create: `crates/op-host-services/src/import_html_url.rs`
- Modify: `crates/op-host-services/src/lib.rs`(声明 `mod import_html_url;`)
- Modify: `crates/op-host-services/src/mcp_serve.rs`(注册,:589 区)
- Modify: `crates/op-host-services/src/mcp_serve/schemas.rs`(schema 数组加一项)
- Modify: `crates/op-host-services/src/mcp_serve/tests.rs`(**精确计数 121→122** + 名单/参数抽查)
- Modify: `crates/op-host-services/Cargo.toml`(加 `op-html = { path = "../op-html" }`)

**Interfaces:**
- Consumes: `op_mcp::{McpTool, ToolOutcome, ToolErrorCode}`、`op_mcp::write_tools::parse_opt_i32`(pub(crate)——**跨 crate 不可见!改从本 crate 内联一个同型私有 helper**,7 行,勿改 op-mcp 可见性)、`root_or_node_id` 同理内联;`EditorCommand::InsertSubtree`;`provider_dial::client_for(EndpointDialPolicy::PublicOnly, url)`、`web_credentials::validate_web_provider_base_url_with_allowlist`(pub(crate) 于本 crate 内,可直接用);`web_image_search::read_capped`(pub(crate))、`ImageJobSlot`;`chat_builtin_http` 超时姿势;`op_html::{import_html_with_resources, HtmlImportOptions}`
- Produces:
  - `pub(crate) struct ImportHtmlUrl; impl McpTool for ImportHtmlUrl`(`name() = "import_html_url"`)+ `pub(crate) fn import_html_url_snapshot() -> ImportHtmlUrl`
  - 参数:`url`(必填)、`x`/`y`(i32,默认 0)、`parent`、`pageId`(与 `import_html` 同义同别名)
- 行为(`call` 内,阻塞式——工具接口是同步的):
  1. `url` 校验:`validate_web_provider_base_url_with_allowlist` 同款筛(http/https、无 userinfo/query 保留 query!——**注意**:网页 URL 常带 query,这里放宽为允许 query/fragment,只保留 scheme + 受限 IP/主机名筛;写一个本模块 `fn screen_import_url(url: &str) -> Result<reqwest::Url, String>`:解析 → scheme http/https → host 非受限(`web_credentials::is_restricted_ip`/`is_restricted_hostname` 逻辑;allowlist 环境变量 `OPENPENCIL_WEB_AI_ENDPOINT_ALLOWLIST` 命中则放行受限主机)。
  2. 单线程 tokio `block_on`(镜像 `remote_image_host.rs:120` 构造)内:`client_for(PublicOnly, url)`(allowlist 命中改 `Trusted`)→ GET 页面,20s 超时,`read_capped(resp, 10 * 1024 * 1024)`(HTML 本体上限 10MiB,对齐子项目 1 输入上限),content-type 含 `text/html` 或体前 512B 嗅探 `<`,否则 `ToolFailed("not an html page")`。
  3. `ImageJobSlot::acquire()` 包住整个抓取会话(拿不到 → `ToolFailed("too many concurrent import jobs")`),资源 fetcher 闭包:同一 runtime `block_on` 单个 GET + `read_capped(resp, 4 * 1024 * 1024)`,同样过 `screen_import_url`(**每个资源 URL 都要筛**——页面可指向内网资源,这是 SSRF 的第二道口)。
  4. `import_html_with_resources(&html, &opts{base_url: Some(最终 URL——用 `resp.url().to_string()` 跟随重定向后的), document_name: None}, Some(&fetcher), None)`(服务端无 skia downscale——transform 传 None,体积由 4MiB 单资源上限兜底)。
  5. nodes 空 → `InvalidArgument`(带首条 warning);x/y 写根 Frame;out:`wrote=true`、`nodeCount`、`sourceUrl`、warnings 拼接 —— 返回 `OkWithCommand(out, InsertSubtree { nodes, parent_id, page_id })`(**镜像 op-mcp/src/import_html_tool.rs 的收尾,先读原文**)。
- 注册:`register_tool!("import_html_url", import_html_url_snapshot());`(import_svg 行旁);schema 条目:

```rust
r#"{"name":"import_html_url","description":"Fetch a web page server-side (SSRF-guarded, public hosts only unless allowlisted) and insert it as editable auto-layout nodes. Static fetch: no JS execution; SPA pages may import incomplete.","inputSchema":{"type":"object","properties":{"filePath":{"type":"string","description":"Optional target .op file path; omit to use the server document"},"url":{"type":"string","description":"http(s) page URL"},"x":{"type":"string","description":"i32 doc-px x offset (default 0)"},"y":{"type":"string","description":"i32 doc-px y offset (default 0)"},"parent":{"type":"string","description":"optional parent node id; empty/0/root omitted = page root"},"pageId":{"type":"string","description":"optional target page id or legacy page index; omitted = active page"}}}}"#,
```

- [ ] **Step 1: 写失败测试(import_html_url.rs 底部)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_import_url_rejects_restricted_hosts() {
        assert!(screen_import_url("https://example.com/page").is_ok());
        assert!(screen_import_url("https://example.com/p?q=1#f").is_ok()); // query allowed
        assert!(screen_import_url("http://127.0.0.1:3000/").is_err());
        assert!(screen_import_url("http://169.254.169.254/meta").is_err());
        assert!(screen_import_url("http://localhost/x").is_err());
        assert!(screen_import_url("ftp://example.com/x").is_err());
        assert!(screen_import_url("https://user:pw@example.com/").is_err()); // userinfo
    }

    #[test]
    fn missing_url_is_typed_error() {
        let out = ImportHtmlUrl.call(&std::collections::BTreeMap::new());
        assert!(matches!(out, op_mcp::ToolOutcome::Err(..)));
    }
}
```

(网络路径不做真实 HTTP 测试——遵循"网络相关测试全部用注入的假 fetcher"的 spec 纪律;抓取逻辑的正确性由 `read_capped`/`client_for` 既有测试与真机冒烟覆盖。计数断言测试在 mcp_serve/tests.rs:21 改 122 并把 `import_html_url` 加进注册名单 + schema 参数抽查 `["filePath","url","parent","pageId"]`。)

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-host-services import_html_url`;`cargo test -p op-host-services mcp_serve`(计数断言此刻应 FAIL,证明注册生效后才绿)。
- [ ] **Step 3: 实现**(allowlist 语义:命中 `base_url_is_explicitly_allowlisted` → 受限主机放行且 dial 用 `Trusted`;DNS 重绑定防护由 `client_for(PublicOnly)` 的 connect-time pin 承担)。
- [ ] **Step 4: 验证** — `cargo test -p op-host-services`(全量,含计数 122)+ `cargo clippy -p op-host-services --all-targets -- -D warnings`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-host-services
git add crates/op-host-services/src/import_html_url.rs crates/op-host-services/src/lib.rs crates/op-host-services/src/mcp_serve.rs crates/op-host-services/src/mcp_serve/schemas.rs crates/op-host-services/src/mcp_serve/tests.rs crates/op-host-services/Cargo.toml
git commit --no-verify -m "feat(mcp): import_html_url tool with ssrf-guarded server-side fetch"
```

---

### Task 7: CLI — URL 路由 + `--out` 本地解析

**Files:**
- Modify: `crates/op-cli/src/main.rs`(`map_import_html` 扩展 + `Command::ImportHtml` 本地变体)
- Create: `crates/op-cli/src/html_cli.rs`(镜像 `figma_cli.rs`)
- Modify: `crates/op-cli/src/tests.rs`(新断言)
- Modify: `crates/op-cli/Cargo.toml`(加 `op-html = { path = "../op-html" }`;**严禁 reqwest**)
- Modify: `crates/op-cli/src/usage.txt`(import:html 行更新)

**Interfaces:**
- Consumes: `op_html::import_html_document`;既有 `map_import_html`(main.rs:702)、`figma_cli.rs` 的 map/run/success-json 三段式、`Command::ImportFigma` 的 enum + dispatch 形状(main.rs:163/:101)
- Produces:
  - `map_import_html` 新逻辑:
    - 位置参数以 `http://`/`https://` 开头 → `tool_call("import_html_url", pairs)`(`url` + x/y/parent/pageId;`--out` 同时出现 → `Err("--out requires a local file; URL import needs a running editor")`)
    - 本地文件 + `--out <x.op>` → `Command::ImportHtml { html_path, out_path }`(本地解析,不需要运行中的编辑器)
    - 本地文件无 `--out` → 现行 `tool_call("import_html", …)` 不变
  - `html_cli.rs`:`pub(super) fn run_import_html(html_path: &str, out_path: &str) -> Result<String, String>` — 读文件 → `import_html_document(源, &opts{document_name: Some(stem)}, Some(&本地 fetcher——同目录相对路径,复用 Task 4 的防越界逻辑:从 html_cli 本地实现同型 `local_resource_fetch`), None)` → `serde_json::to_value(&r.document)` → `std::fs::write` → success JSON `{"ok":true,"filePath":…,"nodeCount":…,"warnings":[…]}`(镜像 figma_cli.rs:24 的 run_import_figma;HTML 无 image_table externalize 需求,跳过该步)
  - usage.txt 行改为:`  op import:html <file.html|url> [--x N] [--y N] [--parent P] [--page PAGE] [--out out.op]`

- [ ] **Step 1: 写失败测试(tests.rs,断言样式与 :200 parse_args 系列同构)**

```rust
#[test]
fn import_html_url_routes_to_url_tool() {
    let args = vec!["import:html".to_string(), "https://example.com/p".to_string()];
    let p = parse_args(&args).expect("parse");
    assert_eq!(p.command, Command::ToolCall {
        tool: "import_html_url".to_string(),
        args: vec![("url".to_string(), "https://example.com/p".to_string())],
    });
}

#[test]
fn import_html_with_out_is_local_command() {
    let args = vec!["import:html".to_string(), "a.html".to_string(),
        "--out".to_string(), "a.op".to_string()];
    let p = parse_args(&args).expect("parse");
    assert_eq!(p.command, Command::ImportHtml {
        html_path: "a.html".to_string(), out_path: "a.op".to_string() });
}

#[test]
fn import_html_url_with_out_errors() {
    let args = vec!["import:html".to_string(), "https://e.com/".to_string(),
        "--out".to_string(), "a.op".to_string()];
    assert!(parse_args(&args).is_err());
}
```

(既有 `import_html_maps_to_tool_call` 测试必须原样保持通过——无 `--out` 的文件路径行为不变。)

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-cli import_html`。
- [ ] **Step 3: 实现**(`Command::ImportHtml` dispatch 臂放 `Command::ImportFigma` 旁 :101;out 缺省不自动推导——`--out` 显式给出才走本地,语义清晰)。
- [ ] **Step 4: 验证** — `cargo test -p op-cli` 全绿 + `cargo build -p op-cli`;端到端一条:`echo '<p>x</p>' > /tmp/t.html && ./target/debug/op import:html /tmp/t.html --out /tmp/t.op && head -c 200 /tmp/t.op`(应是 PenDocument JSON)。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-cli
git add crates/op-cli/src/main.rs crates/op-cli/src/html_cli.rs crates/op-cli/src/tests.rs crates/op-cli/Cargo.toml crates/op-cli/src/usage.txt
git commit --no-verify -m "feat(cli): import:html url routing and --out local parse"
```

---

### Task 8: 收尾验证(全平台)

- [ ] `cargo test -p op-html -p op-mcp -p op-cli -p op-host-services` 全绿
- [ ] `cargo check -p op-host-desktop` + `cargo build -p op-host-desktop`
- [ ] `cargo check --target wasm32-unknown-unknown -p op-html` 与 `-p op-host-web --no-default-features --features web` 均过
- [ ] `cargo clippy -p op-html -p op-host-services -p op-cli --all-targets -- -D warnings`
- [ ] 对照 spec 子项目 2 逐条:粘贴(桌面/web)✓ 文件(桌面/web)✓ URL(服务端工具,CLI/MCP/web 经 /api/mcp 可达)✓ 资源内嵌(fetcher+transform)✓ SSRF ✓;**桌面无独立 URL 输入 UI、web 无 URL 对话框 —— 本计划有意不做 chrome UI(spec 未列 UI 元素;入口=粘贴/拖拽/文件/工具),在最终报告中明示该范围决定**
- [ ] **spec 偏离说明(报告中明示)**:spec 写的是"web 宿主发给 serve-web daemon 的 fetch 端点"——本计划实现为 daemon 内的 MCP 工具 `import_html_url`(浏览器经既有 `/api/mcp` 网关调用,同样是服务端抓取 + SSRF 防护),避免新开 HTTP 面;spec 的 `ResourceFetcher` 类型写"复用 op-figma::image_resolver 类型"——实际在 op-html 本地定义形状相同的别名,避免 op-html 拖上整个 op-figma(kiwi 解析器)依赖
- [ ] 真机冒烟(有显示环境时):粘贴一段带内联样式的 HTML 到桌面画布;`op import:html https://example.com`(需 desktop --serve-web 或 headless MCP 运行中)


