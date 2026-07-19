# HTML 导入子项目 3(真实网页快照链路)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户在自己浏览器里跑一段抽取脚本(devtools 粘贴/书签),用 `getComputedStyle` + 布局坐标生成快照 JSON;OpenPencil 侧把快照转换为**像素级还原的绝对定位** PenNode,经 MCP `import_web_snapshot` 工具与 CLI `op import:snapshot` 导入。SPA/复杂 CSS 页面的正确归宿(spec 子项目 3)。

**Architecture:** 快照 JSON 是稳定契约(version 字段);浏览器脚本零依赖 IIFE,收集元素盒(page 坐标)+ computed style 白名单 + 文本 run 矩形 + 图片 data URL(CORS 污染则保留 URL);Rust 侧 `op-html/src/snapshot.rs` 解析 JSON(serde_json)→ 绝对定位节点树(全部 `LayoutMode::None` + 显式 x/y/w/h),样式换算复用 op-html 既有 color/shadow/radius 解析。产品侧不内嵌浏览器(spec 原决策)。

**Tech Stack:** `serde_json`(op-html 新增,wasm-clean);零依赖浏览器 JS(ES2017,`node --check` 可校验);既有 op-mcp/op-cli 工具样板。

**Spec:** `docs/superpowers/specs/2026-07-17-html-import-design.md` 子项目 3 节。

**依赖:** 本计划在子项目 2 计划(`2026-07-18-html-import-entrypoints.md`)之后执行——工具计数基线 122、`import_html_document` 包装函数、`html_cli.rs` 均由其提供;若顺序变化,计数断言以执行时现状 +1 为准。

## Global Constraints

- 与子项目 1/2 相同:单文件 ≤800 行、op-html wasm32-clean、Conventional Commits(`html`/`mcp`/`cli` scope)、`git commit --no-verify` + 精确路径 add、沙箱无 git 则跳过提交由主会话补。
- 节点上限沿用 **20_000**(`op-html/src/lib.rs` 的 `MAX_OUTPUT_NODES`,快照转换共用);快照 JSON 输入上限 **32 MiB**(data URL 密集,大于 HTML 的 10MiB)。
- 浏览器脚本**零外部依赖、零网络请求**(图片经 canvas 重编码,尺寸上限 2048px 边长);不修改被抽取页面(只读遍历)。
- **快照格式版本必须校验**:`version != 1` → 明确错误,不猜。
- MCP 工具计数断言:子项目 2 完成后为 **122**,本计划加 `import_web_snapshot` → **123**(`op-host-services/src/mcp_serve/tests.rs:21`)。

## 快照 JSON 契约(v1,本计划的核心接口)

```json
{
  "version": 1,
  "source": "https://example.com/page",
  "title": "Page Title",
  "viewport": { "width": 1440, "height": 900 },
  "root": {
    "kind": "element",
    "tag": "body",
    "rect": { "x": 0, "y": 0, "w": 1440, "h": 2400 },
    "styles": { "background-color": "rgb(255, 255, 255)" },
    "children": [
      { "kind": "element", "tag": "div",
        "rect": { "x": 24, "y": 24, "w": 300, "h": 80 },
        "styles": {
          "background-color": "rgba(16, 32, 48, 1)", "border-radius": "8px",
          "box-shadow": "rgba(0, 0, 0, 0.25) 0px 4px 8px 0px",
          "border": "1px solid rgb(255, 0, 0)", "opacity": "1", "overflow": "hidden",
          "background-image": "none", "transform": "none"
        },
        "children": [
          { "kind": "text",
            "rect": { "x": 40, "y": 48, "w": 120, "h": 24 },
            "text": "Hello world",
            "styles": { "color": "rgb(255, 255, 255)", "font-family": "Inter, sans-serif",
              "font-size": "16px", "font-weight": "700", "font-style": "normal",
              "line-height": "24px", "letter-spacing": "normal", "text-align": "left" } }
        ] },
      { "kind": "image", "tag": "img",
        "rect": { "x": 24, "y": 128, "w": 200, "h": 150 },
        "src": "data:image/png;base64,....",
        "styles": { "border-radius": "4px", "object-fit": "cover" } }
    ]
  }
}
```

约定:`rect` 一律**页面绝对坐标**(CSS px,已含滚动偏移);`styles` 只含白名单键且是 computed 值(颜色总是 `rgb()/rgba()` 形式);`kind:"text"` 是文本 run(一个 Text DOM 节点经 Range 测得的盒),`text` 为折叠空白后的内容;`kind:"image"` 的 `src` 是 data URL,CORS 污染时是原 URL 且附 `"tainted": true`;不可见元素(display:none / 零尺寸 / opacity 0 / visibility hidden)**不出现**在快照里(脚本侧过滤)。

---

### Task 1: op-html — snapshot.rs 快照解析与绝对定位转换

**Files:**
- Create: `crates/op-html/src/snapshot.rs`
- Modify: `crates/op-html/src/lib.rs`(`pub mod snapshot;` + re-export)
- Modify: `crates/op-html/Cargo.toml`(加 `serde_json = "1"`)
- Create: `crates/op-html/tests/fixtures/snapshot_v1_sample.json`(上文契约示例存为 fixture,含 element/text/image 三种)

**Interfaces:**
- Produces:
  - `pub fn import_snapshot(json: &str, opts: &HtmlImportOptions) -> HtmlImportResult`(nodes = 单个根 Frame;失败——JSON 非法/version 不符——返回空 nodes + 首条 warning 说明原因,**与 import_html 同样永不 panic**)
  - `pub fn import_snapshot_document(json: &str, opts: &HtmlImportOptions) -> HtmlDocumentResult`(包 PenDocument,name 取 `title`;复用子项目 2 Task 2 的包装逻辑——若该任务尚未落地,则本任务自带同型包装函数)
- 转换规则:
  - 根:Frame,`name` = title 或 "Web Snapshot",`width/height: Number(root.rect.w/h)`,`layout: None`(绝对定位树),fill 取 root `background-color`(默认白)。
  - `kind:"element"` → Frame:`x/y` = 自身 rect 相对**父** rect(`child.rect.x - parent.rect.x`),`width/height: Number`,`layout: None`;样式映射复用既有解析:`background-color`→Solid fill(`parse_css_color` 认 `rgb()/rgba()`)、`background-image` 含 `url(`→`PenFill::Image`(url 原样)含 `-gradient(`→渐变(复用 mapper 的渐变解析——**把 mapper.rs 里的渐变/阴影/border 解析函数提为 `pub(crate)` 供 snapshot.rs 复用,勿复制粘贴**)、`border-radius`→cornerRadius、`box-shadow`→Shadow effects(注意 computed 序:`color offsetX offsetY blur spread`,颜色在前——与作者序不同,解析函数需兼容两种 token 序:逐 token 判定"是颜色还是长度")、`border: <w> <style> <color>`→stroke、`opacity`→base、`overflow:hidden`→clipContent、`transform` 含 `matrix(`→仅提取旋转角(`atan2(b, a)`,弧度→度)写 `rotation`,非旋转矩阵成分忽略 + warning(warn_once)。
  - `kind:"text"` → TextNode:`x/y/w/h`(width/height 用 `Number`),`content: Plain(text)`,字体属性从 styles(`font-size`/`font-weight`(数字串)/`font-family`/`font-style`/`letter-spacing`(`normal`→None)/`line-height`(px→÷font-size 归一倍数)/`text-align`/`color`→fill)。
  - `kind:"image"` → ImageNode:`src` 原样(data URL 或 tainted URL),`object-fit`→objectFit,`border-radius`→cornerRadius;`tainted:true` → warning 一条(汇总计数:"N images kept as remote URLs (CORS-tainted)")。
  - 节点计数超 `MAX_OUTPUT_NODES` → 截断 + warning(复用 lib.rs 的常量,必要时提为 `pub(crate)`)。
- 强调:**绝对定位树**,不做 auto-layout 推断(spec:快照路径=像素还原;"检测 flex 重建 auto-layout"是 spec 明示的后续升级,本计划不做)。

- [ ] **Step 1: 写失败测试(snapshot.rs 底部)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::HtmlImportOptions;
    use jian_ops_schema::node::PenNode;
    use jian_ops_schema::node::container::LayoutMode;

    const SAMPLE: &str = include_str!("../tests/fixtures/snapshot_v1_sample.json");

    #[test]
    fn sample_snapshot_converts_to_absolute_tree() {
        let r = import_snapshot(SAMPLE, &HtmlImportOptions::default());
        assert!(r.nodes.len() == 1, "warnings: {:?}", r.warnings);
        let PenNode::Frame(root) = &r.nodes[0] else { panic!() };
        assert!(matches!(root.container.layout, None | Some(LayoutMode::None)));
        let kids = root.children.as_ref().unwrap();
        let PenNode::Frame(card) = &kids[0] else { panic!("card frame") };
        assert_eq!(card.base.x, Some(24.0));
        assert_eq!(card.base.y, Some(24.0));
        use jian_ops_schema::sizing::SizingBehavior;
        assert!(matches!(card.container.width, Some(SizingBehavior::Number(w)) if w == 300.0));
        let PenNode::Text(t) = &card.children.as_ref().unwrap()[0] else { panic!("text run") };
        assert_eq!(t.base.x, Some(16.0)); // 40 - 24, relative to card
        assert_eq!(t.font_size, Some(16.0));
        assert_eq!(t.line_height, Some(1.5)); // 24px / 16px
        let PenNode::Image(img) = &kids[1] else { panic!("image") };
        assert!(img.src.as_str().starts_with("data:image/png"));
    }

    #[test]
    fn computed_order_box_shadow_parses() {
        let r = import_snapshot(SAMPLE, &HtmlImportOptions::default());
        let PenNode::Frame(root) = &r.nodes[0] else { panic!() };
        let PenNode::Frame(card) = &root.children.as_ref().unwrap()[0] else { panic!() };
        let effects = card.container.effects.as_ref().expect("shadow");
        assert!(matches!(&effects[0], jian_ops_schema::style::PenEffect::Shadow(s)
            if s.offset_y == 4.0 && s.blur == 8.0 && s.color == "#00000040"));
    }

    #[test]
    fn bad_version_and_bad_json_warn_not_panic() {
        let r = import_snapshot("{\"version\":2,\"root\":{}}", &HtmlImportOptions::default());
        assert!(r.nodes.is_empty());
        assert!(r.warnings[0].contains("version"));
        let r2 = import_snapshot("not json", &HtmlImportOptions::default());
        assert!(r2.nodes.is_empty());
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html snapshot`。
- [ ] **Step 3: 实现**(serde 解析用 `serde_json::Value` 手取字段即可,不定义 derive 结构体——契约字段少,Value 走查 + 缺字段容错(缺 styles 当空)更抗脏数据;mapper 解析函数提可见性时保持既有单测不动)。
- [ ] **Step 4: 验证** — `cargo test -p op-html` 全绿 + wasm check + clippy。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src crates/op-html/Cargo.toml crates/op-html/tests
git commit --no-verify -m "feat(html): web snapshot v1 parser producing absolute-positioned nodes"
```

---

### Task 2: 浏览器抽取脚本

**Files:**
- Create: `crates/op-html/assets/snapshot-extractor.js`
- Create: `docs/html-snapshot-import.md`(使用说明,中文可)
- Modify: `crates/op-html/src/snapshot.rs`(`pub const SNAPSHOT_EXTRACTOR_JS: &str = include_str!("../assets/snapshot-extractor.js");` — 让宿主/文档可内嵌分发脚本)

**Interfaces:**
- Produces: 零依赖 IIFE,在任意页面 devtools 控制台粘贴执行后:遍历 `document.body` → 生成契约 v1 JSON → `navigator.clipboard.writeText` + 触发 `snapshot.json` 下载(双保险)+ `console.log` 尺寸统计。
- 脚本行为规格(实现要点,全部落在一个 IIFE 内,ES2017,≤400 行):
  - 跳过:`SCRIPT/STYLE/NOSCRIPT/TEMPLATE/META/LINK/HEAD` 标签、`display:none`/`visibility:hidden`/`opacity==="0"`、零尺寸(`rect.width<0.5||rect.height<0.5`)且无可见子元素的元素。
  - 盒:`el.getBoundingClientRect()` + `window.scrollX/Y` → 页面坐标,round 到 0.01。
  - styles 白名单(元素):`background-color,background-image,border-radius,box-shadow,border,opacity,overflow,transform,object-fit`;文本 run:`color,font-family,font-size,font-weight,font-style,line-height,letter-spacing,text-align`(取自**父元素**的 computed——Text 节点无自身 style)。值为浏览器 computed 串,原样透传(`none`/`normal`/`auto` 也透传,Rust 侧忽略)。
  - 文本 run:遍历元素的直接 Text 子节点,`document.createRange()` `selectNodeContents` → `getBoundingClientRect`;`textContent` 折叠空白后为空则跳过。
  - 图片:`<img>` 完成加载且未跨域污染 → 画到离屏 canvas(最长边 cap 2048,等比)→ `toDataURL("image/png")`;`SecurityError`(tainted)→ `src` 原样 + `tainted:true`。`<svg>` 内联 → `XMLSerializer` → `data:image/svg+xml;base64`。`<canvas>/<video>` → 尝试 `toDataURL`,失败则灰占位标记 `tainted:true`。
  - 深度/规模护栏:元素计数 > 20000 时停止并在 JSON 附 `"truncated": true`;单页 data URL 累计 > 24 MiB 后停止内嵌图片改留 URL(附计数进 console 统计)。
  - 无网络请求、不修改 DOM(离屏 canvas 不插入文档)。
- `docs/html-snapshot-import.md` 内容:三步用法(打开页面 → devtools 粘贴脚本回车 → 得到 snapshot.json;`op import:snapshot snapshot.json` 或 MCP `import_web_snapshot`)、书签化方法(`javascript:` 前缀包裹压缩体的说明,不承诺托管)、已知限制(iframe 内容不进、CORS 图片保留远程 URL、动画取当前帧)。

- [ ] **Step 1: 写"失败测试"**(JS 无测试框架——验收=语法检查+契约 fixture 一致性):先写 `docs/html-snapshot-import.md` 骨架与空的 `snapshot-extractor.js`(只有 `(function(){})();`),确认 `node --check crates/op-html/assets/snapshot-extractor.js` 通过、`cargo test -p op-html snapshot`(include_str 编译)通过 —— 此为基线。
- [ ] **Step 2: 实现脚本全量逻辑**(对照上面规格逐条;JSON 字段名与契约严格一致——Rust 测试的 fixture 就是"契约的可执行断言")。
- [ ] **Step 3: 验证** — `node --check` 通过;`node -e` 驱动一个 jsdom 不可用的场景无意义,改为**人工冒烟指引**写进 docs(在 Chrome 打开 https://example.com,粘贴脚本,把产物 JSON 存为 /tmp/s.json,跑 `cargo run -p op-cli -- import:snapshot /tmp/s.json --out /tmp/s.op`——该 CLI 在 Task 4 落地,冒烟放全计划收尾);`cargo test -p op-html` 全绿。
- [ ] **Step 4: 提交**

```bash
git add crates/op-html/assets crates/op-html/src/snapshot.rs docs/html-snapshot-import.md
git commit --no-verify -m "feat(html): browser snapshot extractor script and usage doc"
```

---

### Task 3: MCP 工具 `import_web_snapshot`

**Files:**
- Create: `crates/op-mcp/src/import_snapshot_tool.rs`
- Create: `crates/op-mcp/src/import_snapshot_tool_tests.rs`
- Modify: `crates/op-mcp/src/lib.rs`(两个 mod 声明,镜像 import_html_tool 的声明形状)
- Modify: `crates/op-host-services/src/mcp_serve.rs`(import + register)
- Modify: `crates/op-host-services/src/mcp_serve/schemas.rs`(加一项)
- Modify: `crates/op-host-services/src/mcp_serve/tests.rs`(**计数 122→123** + 名单/参数抽查)

**Interfaces:**
- Consumes: `op_html::{import_snapshot, HtmlImportOptions}`;op-mcp 既有 `McpTool/ToolOutcome/ToolErrorCode`、`import_html_tool.rs` 的整体形状(参数别名、x/y 应用、InsertSubtree 收尾——**先读原文再镜像**)
- Produces: `pub struct ImportWebSnapshot; impl McpTool`(name `"import_web_snapshot"`)、`pub fn import_web_snapshot_tool() -> ImportWebSnapshot`(注意:**不能**叫 `import_web_snapshot_snapshot`,也别与 op-html 的函数名撞——构造器命名破例用 `_tool` 后缀,注册宏不在乎)
- 参数:`snapshot`(JSON 字符串)或 `snapshotPath`/`snapshot_path`(文件);`x`/`y`/`parent`/`pageId` 同 import_html。nodes 空 → InvalidArgument 带首条 warning。
- schema 条目:

```rust
r#"{"name":"import_web_snapshot","description":"Import a page snapshot JSON produced by the OpenPencil browser extractor script (getComputedStyle + layout boxes) as pixel-accurate absolutely-positioned nodes. The right path for SPA/complex-CSS pages.","inputSchema":{"type":"object","properties":{"filePath":{"type":"string","description":"Optional target .op file path; omit to use the server document"},"snapshot":{"type":"string","description":"snapshot JSON text (contract v1)"},"snapshotPath":{"type":"string","description":"local snapshot .json file path"},"x":{"type":"string","description":"i32 doc-px x offset (default 0)"},"y":{"type":"string","description":"i32 doc-px y offset (default 0)"},"parent":{"type":"string","description":"optional parent node id; empty/0/root omitted = page root"},"pageId":{"type":"string","description":"optional target page id or legacy page index; omitted = active page"}}}}"#,
```

- [ ] **Step 1: 写失败测试(import_snapshot_tool_tests.rs,镜像 import_html_tool_tests.rs 的 use 形状)**

```rust
use std::collections::BTreeMap;

use super::import_snapshot_tool::{import_web_snapshot_tool, ImportWebSnapshot};
use super::{EditorCommand, McpTool, ToolOutcome};

const SAMPLE: &str = include_str!("../../op-html/tests/fixtures/snapshot_v1_sample.json");

#[test]
fn snapshot_import_returns_insert_subtree() {
    let tool: ImportWebSnapshot = import_web_snapshot_tool();
    let mut args = BTreeMap::new();
    args.insert("snapshot".to_string(), SAMPLE.to_string());
    args.insert("x".to_string(), "50".to_string());
    let ToolOutcome::OkWithCommand(map, EditorCommand::InsertSubtree { nodes, .. }) = tool.call(&args)
        else { panic!("expected OkWithCommand(InsertSubtree)") };
    assert_eq!(map.get("wrote").map(String::as_str), Some("true"));
    assert_eq!(nodes.len(), 1);
}

#[test]
fn invalid_snapshot_is_typed_error() {
    let tool = import_web_snapshot_tool();
    let mut args = BTreeMap::new();
    args.insert("snapshot".to_string(), "{\"version\":9}".to_string());
    assert!(matches!(tool.call(&args), ToolOutcome::Err(..)));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-mcp import_snapshot`;`cargo test -p op-host-services mcp_serve`(计数断言在注册后才应转绿)。
- [ ] **Step 3: 实现 + 注册**。
- [ ] **Step 4: 验证** — `cargo test -p op-mcp` 与 `cargo test -p op-host-services` 全绿。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-mcp -p op-host-services
git add crates/op-mcp/src/import_snapshot_tool.rs crates/op-mcp/src/import_snapshot_tool_tests.rs crates/op-mcp/src/lib.rs crates/op-host-services/src/mcp_serve.rs crates/op-host-services/src/mcp_serve/schemas.rs crates/op-host-services/src/mcp_serve/tests.rs
git commit --no-verify -m "feat(mcp): import_web_snapshot tool for pixel-accurate page snapshots"
```

---

### Task 4: CLI `op import:snapshot`

**Files:**
- Modify: `crates/op-cli/src/main.rs`(dispatch 臂 + `map_import_snapshot`;`--out` 本地臂 `Command::ImportSnapshot`)
- Modify: `crates/op-cli/src/html_cli.rs`(`run_import_snapshot`——若子项目 2 Task 7 的 html_cli.rs 未落地,本任务创建该文件并只含本函数)
- Modify: `crates/op-cli/src/tests.rs`、`crates/op-cli/src/usage.txt`

**Interfaces:**
- Consumes: `op_html::{import_snapshot_document, HtmlImportOptions}`;main.rs 既有 helper(`required_pos/pair/flag_value/push_file_path/tool_call`)
- Produces:
  - `op import:snapshot <file.json> [--x N] [--y N] [--parent P] [--page PAGE]` → `tool_call("import_web_snapshot", [("snapshotPath", path), …])`
  - `op import:snapshot <file.json> --out <x.op>` → `Command::ImportSnapshot { json_path, out_path }` → `run_import_snapshot`:读文件 → `import_snapshot_document` → PenDocument JSON 写盘 → success JSON(镜像 run_import_html/图 figma 三段式)
  - usage.txt 加行:`  op import:snapshot <snapshot.json> [--x N] [--y N] [--parent P] [--page PAGE] [--out out.op]`

- [ ] **Step 1: 写失败测试(tests.rs)**

```rust
#[test]
fn import_snapshot_maps_to_tool_call() {
    let args = vec!["import:snapshot".to_string(), "s.json".to_string(),
        "--page".to_string(), "p1".to_string()];
    let p = parse_args(&args).expect("parse");
    assert_eq!(p.command, Command::ToolCall {
        tool: "import_web_snapshot".to_string(),
        args: vec![
            ("snapshotPath".to_string(), "s.json".to_string()),
            ("pageId".to_string(), "p1".to_string()),
        ],
    });
}

#[test]
fn import_snapshot_with_out_is_local() {
    let args = vec!["import:snapshot".to_string(), "s.json".to_string(),
        "--out".to_string(), "s.op".to_string()];
    let p = parse_args(&args).expect("parse");
    assert_eq!(p.command, Command::ImportSnapshot {
        json_path: "s.json".to_string(), out_path: "s.op".to_string() });
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-cli import_snapshot`。
- [ ] **Step 3: 实现**。
- [ ] **Step 4: 验证** — `cargo test -p op-cli` 全绿;端到端:`./target/debug/op import:snapshot crates/op-html/tests/fixtures/snapshot_v1_sample.json --out /tmp/s.op && head -c 200 /tmp/s.op`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-cli
git add crates/op-cli/src/main.rs crates/op-cli/src/html_cli.rs crates/op-cli/src/tests.rs crates/op-cli/src/usage.txt
git commit --no-verify -m "feat(cli): import:snapshot subcommand for web page snapshots"
```

---

### Task 5: 收尾验证

- [ ] `cargo test -p op-html -p op-mcp -p op-cli -p op-host-services` 全绿(host-services 计数 123)
- [ ] `cargo check --target wasm32-unknown-unknown -p op-html`、`cargo clippy -p op-html -p op-mcp -p op-cli -p op-host-services --all-targets -- -D warnings`
- [ ] `node --check crates/op-html/assets/snapshot-extractor.js`
- [ ] 对照 spec 子项目 3:快照格式(v1 契约)✓ 浏览器脚本 ✓ 快照→绝对定位转换 ✓ 复用样式映射 ✓;"检测 flex 重建 auto-layout"为 spec 明示后续升级,不在本计划(报告中明示)
- [ ] 人工冒烟(有浏览器环境时):按 docs/html-snapshot-import.md 三步走一遍 example.com

