# HTML 导入子项目 1(op-html 核心转换器)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建 `crates/op-html`,把 HTML+手写 CSS 子集级联转换为可编辑的 auto-layout `PenNode` 树,并接上 MCP `import_html` 工具与 CLI `op import:html` 子命令。

**Architecture:** html5ever 解 DOM → 手写 CSS 子集(声明解析/shorthand 展开/选择器匹配/级联+继承)→ mapper 把样式化 DOM 映射为 Frame/Text/Image/控件节点。纯转换库,不碰网络与编辑器状态;MCP 工具解析后经 `EditorCommand::InsertSubtree` 交给编辑器,editor-core 不依赖 op-html。

**Tech Stack:** Rust;`html5ever = "0.27"` + `markup5ever_rcdom = "0.3"`(Servo 系,wasm 兼容)+ `base64 = "0.22"`(SVG data-URL)+ `jian-ops-schema`(workspace path 依赖)。

**Spec:** `docs/superpowers/specs/2026-07-17-html-import-design.md`(子项目 1 部分)。

## Global Constraints

- 单文件 ≤ **800 行**,超限拆分;一文件一职责;`.rs` 用 snake_case;源码注释一律英文。
- `op-html` 保持 **wasm32-clean**:依赖仅 `html5ever`/`markup5ever_rcdom`/`base64`/`jian-ops-schema`,不引入网络/图片解码/线程。
- 默认值(spec 已确认):根 Frame 宽度 **1440**;rem/em 基准 **16px**;内联 SVG → **data-URL Image 降级**。
- 防御上限:输入 HTML ≤ **10MB**(超出截断+warning);产出节点 ≤ **20_000**(到达即停止映射+warning)。
- **永不硬失败**:任何输入都返回 `HtmlImportResult`;问题进 `warnings`。
- Conventional Commits:op-html 任务用 `feat(html): …`(新 scope,对齐 `figma` scope 先例);MCP 任务 `feat(mcp)`;CLI 任务 `feat(cli)`。
- **本机 pre-commit 现状**:仓库 rustfmt.toml 用 nightly-only 选项,本机 stable rustfmt 会在两个**未触碰的legacy 文件**(`op-opmerge/src/tests.rs`、`jian-ops-schema/src/image_table.rs`)上报格式差异,导致任何提交被钩子挡下。每个任务提交前先跑 `cargo fmt -p <本任务 crate>`,然后用 `git commit --no-verify` 提交;**不要**跑 `cargo fmt --all`(会改动无关文件)。
- 工作区根 `Cargo.toml` 的 `members = ["crates/*"]` 是 glob,新增 crate **无需**改根清单;但 `default-members` 不含 op-html,构建/测试须显式 `-p op-html`。
- op-cli 的 `Cargo.toml`/`figma_cli.rs` 有他人未提交的本地改动(image externalize),**不要触碰或回滚**;本计划只新增代码行。

---

### Task 1: 脚手架 — crate、公开 API 类型、空输入行为

**Files:**
- Create: `crates/op-html/Cargo.toml`
- Create: `crates/op-html/src/lib.rs`

**Interfaces:**
- Produces(后续所有任务依赖):
  - `pub struct HtmlImportOptions { pub viewport_width: f64, pub base_font_size: f64, pub document_name: Option<String> }`(`Default` = 1440.0 / 16.0 / None)
  - `pub struct HtmlImportResult { pub nodes: Vec<jian_ops_schema::node::PenNode>, pub warnings: Vec<String> }`
  - `pub fn import_html(source: &str, opts: &HtmlImportOptions) -> HtmlImportResult`

- [ ] **Step 1: 写 Cargo.toml**

```toml
[package]
name = "op-html"
version = "0.1.0"
edition = "2021"
description = "HTML/CSS-subset importer: converts HTML into editable PenNode trees"
license = "MIT"

[dependencies]
html5ever = "0.27"
markup5ever_rcdom = "0.3"
base64 = "0.22"
jian-ops-schema = { path = "../../vendor/jian/crates/jian-ops-schema" }
```

- [ ] **Step 2: 写失败测试(lib.rs 底部 tests 模块)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_nodes_and_a_warning() {
        let r = import_html("", &HtmlImportOptions::default());
        assert!(r.nodes.is_empty());
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("no importable content"));
    }

    #[test]
    fn options_default_values() {
        let o = HtmlImportOptions::default();
        assert_eq!(o.viewport_width, 1440.0);
        assert_eq!(o.base_font_size, 16.0);
        assert!(o.document_name.is_none());
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p op-html`
Expected: 编译失败(`import_html` 未定义)。

- [ ] **Step 4: 最小实现(lib.rs)**

```rust
//! HTML → PenNode importer (structured path, CSS-subset cascade).

use jian_ops_schema::node::PenNode;

pub struct HtmlImportOptions {
    pub viewport_width: f64,
    pub base_font_size: f64,
    pub document_name: Option<String>,
}

impl Default for HtmlImportOptions {
    fn default() -> Self {
        Self { viewport_width: 1440.0, base_font_size: 16.0, document_name: None }
    }
}

pub struct HtmlImportResult {
    pub nodes: Vec<PenNode>,
    pub warnings: Vec<String>,
}

pub fn import_html(source: &str, opts: &HtmlImportOptions) -> HtmlImportResult {
    let _ = opts;
    let mut warnings = Vec::new();
    if source.trim().is_empty() {
        warnings.push("no importable content: input HTML is empty".to_string());
        return HtmlImportResult { nodes: Vec::new(), warnings };
    }
    // Pipeline lands in later tasks; non-empty input is wired up in Task 11.
    warnings.push("no importable content: importer pipeline not yet implemented".to_string());
    HtmlImportResult { nodes: Vec::new(), warnings }
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p op-html`
Expected: 2 passed。同时跑 `cargo check --target wasm32-unknown-unknown -p op-html` 确认 wasm 干净(html5ever/base64 均应通过)。

- [ ] **Step 6: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html
git commit --no-verify -m "feat(html): scaffold op-html importer crate with public API"
```

---

### Task 2: dom.rs — html5ever 解析为精简 DOM

**Files:**
- Create: `crates/op-html/src/dom.rs`
- Modify: `crates/op-html/src/lib.rs`(加 `pub mod dom;`)

**Interfaces:**
- Produces:
  - `pub enum DomNode { Element(DomElement), Text(String) }`
  - `pub struct DomElement { pub tag: String, pub attrs: Vec<(String, String)>, pub children: Vec<DomNode> }`
    - `impl DomElement { pub fn attr(&self, name: &str) -> Option<&str>; pub fn classes(&self) -> Vec<&str>; pub fn id(&self) -> Option<&str>; }`
  - `pub struct ParsedDom { pub body: Vec<DomNode>, pub style_blocks: Vec<String>, pub title: Option<String> }`
  - `pub fn parse_dom(source: &str) -> ParsedDom`
- 行为约定:`tag` 统一小写;`<style>` 的文本内容收进 `style_blocks`(不进 body 树);`<title>` 文本进 `title`;`script/noscript/template/meta/link/head` 整棵丢弃;注释丢弃;片段输入(无 html/body 包裹)也要能解析出 body 级子节点(html5ever 的 document 解析会自动补 body)。

- [ ] **Step 1: 写失败测试(dom.rs 底部)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fragment_without_wrapper() {
        let d = parse_dom("<div class=\"a b\" id=\"x\"><p>hi</p></div>");
        assert_eq!(d.body.len(), 1);
        let DomNode::Element(div) = &d.body[0] else { panic!("expected element") };
        assert_eq!(div.tag, "div");
        assert_eq!(div.classes(), vec!["a", "b"]);
        assert_eq!(div.id(), Some("x"));
        let DomNode::Element(p) = &div.children[0] else { panic!("expected p") };
        assert_eq!(p.tag, "p");
        assert!(matches!(&p.children[0], DomNode::Text(t) if t == "hi"));
    }

    #[test]
    fn collects_style_blocks_and_title_drops_script() {
        let d = parse_dom(
            "<html><head><title>Page</title><style>.a{color:red}</style></head>\
             <body><script>evil()</script><span>ok</span></body></html>",
        );
        assert_eq!(d.title.as_deref(), Some("Page"));
        assert_eq!(d.style_blocks, vec![".a{color:red}".to_string()]);
        assert_eq!(d.body.len(), 1); // script dropped, span kept
    }

    #[test]
    fn tolerates_dirty_html() {
        let d = parse_dom("<div><b>unclosed<div>next</div>");
        assert!(!d.body.is_empty()); // html5ever never fails
    }

    #[test]
    fn style_inside_body_is_still_collected() {
        let d = parse_dom("<div><style>p{margin:0}</style><p>x</p></div>");
        assert_eq!(d.style_blocks, vec!["p{margin:0}".to_string()]);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p op-html dom`
Expected: 编译失败(模块不存在)。

- [ ] **Step 3: 实现 parse_dom**

用 `html5ever::parse_document` + `markup5ever_rcdom::RcDom` 解析,然后递归把 rcdom `Handle` 转成自有 `DomNode`,同时收集 style/title:

```rust
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

const DROP_TAGS: &[&str] = &["script", "noscript", "template", "meta", "link", "head"];

pub fn parse_dom(source: &str) -> ParsedDom {
    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut source.as_bytes())
        .expect("reading from &[u8] cannot fail");
    let mut out = ParsedDom { body: Vec::new(), style_blocks: Vec::new(), title: None };
    // Walk document → <html>: harvest <head> (title/style blocks), then convert
    // each <body> child via `convert`, pushing Some results into out.body.
    walk_document(&dom.document, &mut out);
    out
}
```

实现要点(转换函数签名 `fn convert(h: &Handle, out: &mut ParsedDom) -> Option<DomNode>`):

- `NodeData::Element { name, attrs, .. }`:`tag = name.local.to_string().to_lowercase()`。
  - `tag == "style"` → 拼接其文本子节点推入 `out.style_blocks`,返回 None。
  - `tag == "title"` → 文本进 `out.title`,返回 None。
  - `DROP_TAGS.contains(tag)` → 返回 None(但 head 的子节点里 style/title 要先被上面两条收走——按"先递归 head 专门收集,再转换 body"组织:找到 body Handle 后只对其 children 调 convert;对 head 单独走 harvest 函数)。
  - 其余:attrs 转 `Vec<(String,String)>`(名字小写),children 逐个 convert(body 内出现的 `<style>` 同样被收集并从树中剔除)。
- `NodeData::Text { contents }`:borrow 后 to_string;**不在此处 trim**(空白折叠是 text mapper 的职责),但纯空白文本(`trim().is_empty()`)直接丢弃以减树。
- `NodeData::Comment/Doctype/ProcessingInstruction` → None。
- helper:`attr()` 线性查 `attrs`;`classes()` = `attr("class")` 按空白 split;`id()` = `attr("id")`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p op-html dom`
Expected: 4 passed。

- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): parse html5ever dom into simplified tree with style/title harvest"
```

---

### Task 3: color.rs + length.rs — 颜色与长度单位

**Files:**
- Create: `crates/op-html/src/color.rs`
- Create: `crates/op-html/src/length.rs`
- Modify: `crates/op-html/src/lib.rs`(加 `pub mod color; pub mod length;`)

**Interfaces:**
- Produces:
  - `pub fn parse_css_color(v: &str) -> Option<String>` — 输出规范化 `#rrggbb` 或带 alpha 的 `#rrggbbaa`(小写)。支持:`#rgb/#rgba/#rrggbb/#rrggbbaa`、`rgb()/rgba()`(逗号或空格分隔,alpha 0-1 或 %)、`hsl()/hsla()`、`transparent`(→ `#00000000`)、常用命名色(见下)。不认识 → None。
  - `pub struct LengthCtx { pub font_size: f64, pub root_font_size: f64, pub viewport_w: f64, pub viewport_h: f64 }`
  - `pub enum CssLength { Px(f64), Percent(f64) }`
  - `pub fn parse_length(v: &str, ctx: &LengthCtx) -> Option<CssLength>` — 支持 `px/em/rem/%/vw/vh/pt`(pt×4/3)与裸 `0`;em 基于 `ctx.font_size`,rem 基于 `ctx.root_font_size`,vw/vh 基于 viewport。
- 命名色表(至少):black white red green blue yellow orange purple pink gray grey silver transparent + `currentcolor`(→ None,由调用方回退到 computed color)。

- [ ] **Step 1: 写失败测试**

```rust
// color.rs tests
#[test]
fn hex_forms() {
    assert_eq!(parse_css_color("#FA3").as_deref(), Some("#ffaa33"));
    assert_eq!(parse_css_color("#ffaa33").as_deref(), Some("#ffaa33"));
    assert_eq!(parse_css_color("#ffaa3380").as_deref(), Some("#ffaa3380"));
}
#[test]
fn rgb_hsl_named() {
    assert_eq!(parse_css_color("rgb(255, 0, 0)").as_deref(), Some("#ff0000"));
    assert_eq!(parse_css_color("rgba(0,0,0,0.5)").as_deref(), Some("#00000080"));
    assert_eq!(parse_css_color("hsl(0, 100%, 50%)").as_deref(), Some("#ff0000"));
    assert_eq!(parse_css_color("white").as_deref(), Some("#ffffff"));
    assert_eq!(parse_css_color("transparent").as_deref(), Some("#00000000"));
    assert!(parse_css_color("var(--x)").is_none());
}

// length.rs tests
#[test]
fn units() {
    let ctx = LengthCtx { font_size: 20.0, root_font_size: 16.0, viewport_w: 1440.0, viewport_h: 900.0 };
    assert!(matches!(parse_length("24px", &ctx), Some(CssLength::Px(v)) if v == 24.0));
    assert!(matches!(parse_length("1.5em", &ctx), Some(CssLength::Px(v)) if v == 30.0));
    assert!(matches!(parse_length("2rem", &ctx), Some(CssLength::Px(v)) if v == 32.0));
    assert!(matches!(parse_length("50%", &ctx), Some(CssLength::Percent(v)) if v == 50.0));
    assert!(matches!(parse_length("10vw", &ctx), Some(CssLength::Px(v)) if v == 144.0));
    assert!(matches!(parse_length("0", &ctx), Some(CssLength::Px(v)) if v == 0.0));
    assert!(parse_length("auto", &ctx).is_none());
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html color length`,编译失败。

- [ ] **Step 3: 实现** — 纯字符串处理:hex 解析用 `u8::from_str_radix`;rgb/hsl 剥括号按 `,`/空白切分;hsl→rgb 标准公式;alpha 四舍五入两位 hex,`==255` 时省略 aa 段。length:strip 后缀匹配 + `f64::parse`。

- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html color length`。

- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): css color and length unit parsing"
```

---

### Task 4: css/declarations.rs — 声明解析与 shorthand 展开

**Files:**
- Create: `crates/op-html/src/css/mod.rs`(`pub mod declarations; pub mod selectors; pub mod cascade;` — selectors/cascade 先留空文件占位,内容在 Task 5/6)
- Create: `crates/op-html/src/css/declarations.rs`
- Modify: `crates/op-html/src/lib.rs`(加 `pub mod css;`)

**Interfaces:**
- Produces:
  - `pub struct Declaration { pub name: String, pub value: String, pub important: bool }`
  - `pub fn parse_declarations(block: &str) -> Vec<Declaration>` — 输入 `{}` 内文本或 `style="..."` 内容;按 `;` 切分(但不切开 `url(a;b)` 括号内的分号);属性名小写;`!important` 剥出置 flag;shorthand 展开为 longhand(展开后仍保持源顺序)。
- 展开规则:
  - `margin`/`padding`:1/2/3/4 值 → `-top/-right/-bottom/-left` 四个 longhand(CSS 标准顺序)。
  - `border`:`<width> <style> <color>` 任意序 → `border-width`/`border-style`/`border-color`(统一四边)。`border-top` 等单边 → `border-top-width` 等(仅 width/color;style 丢弃)。
  - `background`:值内含 `url(` → `background-image`;含 `-gradient(` → `background-image`;其余整体尝试作为颜色 → `background-color`(多分量时取第一个能被 `parse_css_color` 认出的 token)。
  - `font`:best-effort 抽 `italic`→`font-style`、`bold|100..900`→`font-weight`、首个带单位 token→`font-size`、`/` 后→`line-height`、其余尾部→`font-family`。
  - `gap`:1 值→`gap`;2 值→取第一个(row-gap)为 `gap` 并丢弃第二个。
  - `flex`:`flex: N` / `flex: N N N` → `flex-grow` 取第一个数。
  - `text-decoration`:含 `underline`→`text-decoration-line:underline`;含 `line-through`→`text-decoration-line:line-through`。
  - `border-radius`:1-4 值 → `border-radius`(保持原值,4 角展开留给 mapper,因为要按 `[tl,tr,br,bl]` 转 `CornerRadius::PerCorner`)。
  - 其他属性原样保留(未知属性由 cascade/mapper 静默忽略)。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn get<'a>(d: &'a [Declaration], n: &str) -> Option<&'a str> {
        d.iter().rev().find(|x| x.name == n).map(|x| x.value.as_str())
    }

    #[test]
    fn margin_shorthand_expands() {
        let d = parse_declarations("margin: 10px 20px");
        assert_eq!(get(&d, "margin-top"), Some("10px"));
        assert_eq!(get(&d, "margin-right"), Some("20px"));
        assert_eq!(get(&d, "margin-bottom"), Some("10px"));
        assert_eq!(get(&d, "margin-left"), Some("20px"));
    }

    #[test]
    fn border_and_background() {
        let d = parse_declarations("border: 1px solid #000; background: #fff url(x.png)");
        assert_eq!(get(&d, "border-width"), Some("1px"));
        assert_eq!(get(&d, "border-color"), Some("#000"));
        assert_eq!(get(&d, "background-image"), Some("url(x.png)"));
        assert_eq!(get(&d, "background-color"), Some("#fff"));
    }

    #[test]
    fn important_flag_and_case() {
        let d = parse_declarations("COLOR: red !important");
        assert_eq!(d[0].name, "color");
        assert_eq!(d[0].value, "red");
        assert!(d[0].important);
    }

    #[test]
    fn gradient_goes_to_background_image() {
        let d = parse_declarations("background: linear-gradient(90deg, #000, #fff)");
        assert_eq!(get(&d, "background-image"), Some("linear-gradient(90deg, #000, #fff)"));
        assert_eq!(get(&d, "background-color"), None);
    }

    #[test]
    fn flex_and_font() {
        let d = parse_declarations("flex: 1; font: italic 700 18px/1.4 Inter, sans-serif");
        assert_eq!(get(&d, "flex-grow"), Some("1"));
        assert_eq!(get(&d, "font-style"), Some("italic"));
        assert_eq!(get(&d, "font-weight"), Some("700"));
        assert_eq!(get(&d, "font-size"), Some("18px"));
        assert_eq!(get(&d, "line-height"), Some("1.4"));
        assert_eq!(get(&d, "font-family"), Some("Inter, sans-serif"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html declarations`。
- [ ] **Step 3: 实现**(分号切分注意括号深度计数;每个 shorthand 一个私有 `expand_*` 函数,主函数 match 属性名分发)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html declarations`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): css declaration parsing with shorthand expansion"
```

---

### Task 5: css/selectors.rs — 选择器子集与匹配

**Files:**
- Create: `crates/op-html/src/css/selectors.rs`(替换 Task 4 的空占位)
- Test: 同文件底部 `mod tests`

**Interfaces:**
- Consumes: `crate::dom::DomElement`
- Produces:
  - `pub struct CompoundSelector { pub tag: Option<String>, pub id: Option<String>, pub classes: Vec<String> }`
  - `pub struct Selector { pub compounds: Vec<CompoundSelector> }` — 后代组合链,最右为目标元素
  - `pub fn parse_selector_list(s: &str) -> Vec<Selector>` — 逗号分组;含不支持记号(`>`, `+`, `~`, `[`, `:`, `*`)的整条选择器丢弃(返回列表里没有它)
  - `pub fn specificity(sel: &Selector) -> (u32, u32, u32)` — (id 数, class 数, tag 数)
  - `pub fn matches(sel: &Selector, path: &[&DomElement]) -> bool` — `path` 为根→自身的元素链;最右 compound 必须匹配 `path.last()`,其余 compound 依序匹配某个更早祖先(标准后代匹配,从右往左贪心)

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::DomElement;

    fn el(tag: &str, class: &str, id: &str) -> DomElement {
        let mut attrs = Vec::new();
        if !class.is_empty() { attrs.push(("class".into(), class.into())); }
        if !id.is_empty() { attrs.push(("id".into(), id.into())); }
        DomElement { tag: tag.into(), attrs, children: Vec::new() }
    }

    #[test]
    fn parses_and_scores() {
        let sels = parse_selector_list("div.card, #hero .title, p");
        assert_eq!(sels.len(), 3);
        assert_eq!(specificity(&sels[0]), (0, 1, 1));
        assert_eq!(specificity(&sels[1]), (1, 1, 0));
        assert_eq!(specificity(&sels[2]), (0, 0, 1));
    }

    #[test]
    fn unsupported_selectors_are_dropped() {
        let sels = parse_selector_list("a:hover, div > p, .ok");
        assert_eq!(sels.len(), 1);
        assert_eq!(sels[0].compounds[0].classes, vec!["ok".to_string()]);
    }

    #[test]
    fn descendant_matching() {
        let hero = el("section", "", "hero");
        let mid = el("div", "card", "");
        let title = el("h2", "title", "");
        let path: Vec<&DomElement> = vec![&hero, &mid, &title];
        let sel = &parse_selector_list("#hero .title")[0];
        assert!(matches(sel, &path));
        let sel2 = &parse_selector_list("#hero .card .title")[0];
        assert!(matches(sel2, &path));
        let sel3 = &parse_selector_list("#other .title")[0];
        assert!(!matches(sel3, &path));
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html selectors`。
- [ ] **Step 3: 实现**(compound 解析:按 `#`/`.` 切 token;matches:从右往左,目标 compound 精确匹配尾元素,其余在剩余祖先里从后往前找)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html selectors`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): selector subset parsing, specificity and descendant matching"
```

---

### Task 6: css/cascade.rs — 样式表解析、UA 默认、级联与继承

**Files:**
- Create: `crates/op-html/src/css/cascade.rs`(替换空占位)

**Interfaces:**
- Consumes: Task 4 `Declaration/parse_declarations`,Task 5 `Selector/parse_selector_list/specificity/matches`,Task 3 `LengthCtx/parse_length`
- Produces:
  - `pub struct StyleRule { pub selector: Selector, pub declarations: Vec<Declaration>, pub order: usize }`
  - `pub fn parse_stylesheet(css: &str, first_order: usize) -> (Vec<StyleRule>, Vec<String>)` — 第二返回值是 warnings;剥 `/* */` 注释;`@media`/`@keyframes`/`@font-face` 等 @-规则整块跳过(嵌套大括号配平)并各记一条 warning(`"@media rules ignored (import viewport applies)"` 等,同类只记一次)
  - `pub const UA_STYLESHEET: &str` — 内容见 Step 3
  - `pub struct ComputedStyle { pub props: std::collections::BTreeMap<String, String>, pub font_size: f64 }`
    - `impl ComputedStyle { pub fn get(&self, name: &str) -> Option<&str> }`
  - `pub fn compute_style(path: &[&crate::dom::DomElement], rules: &[StyleRule], parent: Option<&ComputedStyle>, root_font_size: f64) -> ComputedStyle`
- 级联顺序(后者胜):UA 规则(order 最小)→ 作者规则按 (specificity, order) 稳定排序 → inline `style=""` → 以上所有 `!important` 声明再按同序覆盖一遍。
- 继承:`color/font-family/font-size/font-weight/font-style/line-height/letter-spacing/text-align` 八个属性,元素未声明时取 `parent` 的值。
- `font_size`:声明值经 `parse_length`(em 相对 parent.font_size,rem 相对 root)解析成 px 存入 `font_size` 字段;未声明继承 parent,根默认 root_font_size。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::DomElement;

    fn el(tag: &str, class: &str, style: &str) -> DomElement {
        let mut attrs = Vec::new();
        if !class.is_empty() { attrs.push(("class".into(), class.into())); }
        if !style.is_empty() { attrs.push(("style".into(), style.into())); }
        DomElement { tag: tag.into(), attrs, children: Vec::new() }
    }

    #[test]
    fn specificity_and_order_win() {
        let (rules, _) = parse_stylesheet(
            "p { color: #111111 } .hot { color: #ff0000 } p { margin-top: 4px }", 100);
        let p = el("p", "hot", "");
        let path = [&p];
        let c = compute_style(&path, &rules, None, 16.0);
        assert_eq!(c.get("color"), Some("#ff0000")); // class beats tag
        assert_eq!(c.get("margin-top"), Some("4px"));
    }

    #[test]
    fn inline_beats_rules_but_important_beats_inline() {
        let (rules, _) = parse_stylesheet(".a { color: #00ff00 !important }", 100);
        let d = el("div", "a", "color: #0000ff");
        let path = [&d];
        let c = compute_style(&path, &rules, None, 16.0);
        assert_eq!(c.get("color"), Some("#00ff00"));
    }

    #[test]
    fn inheritance_and_font_size_units() {
        let (rules, _) = parse_stylesheet("div { color: #333333; font-size: 20px }", 100);
        let parent_el = el("div", "", "");
        let parent = compute_style(&[&parent_el], &rules, None, 16.0);
        assert_eq!(parent.font_size, 20.0);
        let child_el = el("span", "", "font-size: 1.5em");
        let child = compute_style(&[&parent_el, &child_el], &rules, Some(&parent), 16.0);
        assert_eq!(child.get("color"), Some("#333333")); // inherited
        assert_eq!(child.font_size, 30.0); // em resolves against parent
    }

    #[test]
    fn at_rules_skipped_with_warning() {
        let (rules, warnings) =
            parse_stylesheet("@media (max-width:600px){ p{color:red} } p{color:#222222}", 0);
        assert_eq!(rules.len(), 1);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn ua_defaults_apply() {
        let (ua, _) = parse_stylesheet(UA_STYLESHEET, 0);
        let h1 = el("h1", "", "");
        let c = compute_style(&[&h1], &ua, None, 16.0);
        assert_eq!(c.font_size, 32.0);
        assert_eq!(c.get("font-weight"), Some("700"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html cascade`。

- [ ] **Step 3: 实现**

UA 样式表常量(参与级联,order 从 0 开始,作者表 `first_order` 从 1000 开始):

```rust
pub const UA_STYLESHEET: &str = "\
body{font-size:16px;color:#111111}\
h1{font-size:32px;font-weight:700;margin:21px 0}\
h2{font-size:24px;font-weight:700;margin:20px 0}\
h3{font-size:19px;font-weight:700;margin:18px 0}\
h4{font-size:16px;font-weight:700;margin:21px 0}\
h5{font-size:13px;font-weight:700;margin:22px 0}\
h6{font-size:11px;font-weight:700;margin:24px 0}\
p{margin:16px 0}\
ul,ol{margin:16px 0;padding:0 0 0 40px}\
b,strong{font-weight:700}\
i,em{font-style:italic}\
u{text-decoration:underline}\
s,del,strike{text-decoration:line-through}\
a{color:#0066cc;text-decoration:underline}\
code,pre{font-family:monospace}\
hr{margin:8px 0}";
```

`compute_style` 算法:收集所有 `matches(rule.selector, path)` 的规则 → 稳定排序 (specificity, order) → 顺序写入 map(普通声明);inline `style` 属性 `parse_declarations` 后写入;再把(规则序+inline)中 `important` 的声明按同序补写一遍。最后处理继承八属性与 `font_size` 解析。

- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html cascade`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): stylesheet parsing, ua defaults, cascade and inheritance"
```

---

### Task 7: mapper.rs — 容器与视觉样式映射

**Files:**
- Create: `crates/op-html/src/mapper.rs`
- Modify: `crates/op-html/src/lib.rs`(加 `pub mod mapper;`)

**Interfaces:**
- Consumes: Task 2 `DomNode/DomElement`,Task 6 `ComputedStyle/StyleRule/compute_style`,Task 3 颜色/长度
- Produces:
  - `pub struct MapCtx<'a> { pub opts: &'a HtmlImportOptions, pub rules: &'a [StyleRule], pub warnings: Vec<String>, pub next_id: u32, pub node_count: usize }`
    - `impl MapCtx { pub fn generate_id(&mut self) -> String /* format!("html_{n}") */; pub fn warn_once(&mut self, msg: &str); }`
  - `pub fn map_element(ctx: &mut MapCtx, path: &[&DomElement], parent_style: Option<&ComputedStyle>) -> Option<PenNode>` — 元素分发入口(文本/特殊元素分支在 Task 9/10 落地,本任务先只做容器)
  - `pub fn container_props_from(style: &ComputedStyle, ctx: &mut MapCtx) -> jian_ops_schema::node::container::ContainerProps`
- 映射规则(spec"映射规则/布局+样式"节):
  - `display:flex`:`flex-direction` 缺省或 `row` → `LayoutMode::Horizontal`,`column` → `Vertical`;非 flex 容器一律 `Vertical`。`display:none` → 整棵跳过(返回 None,无 warning)。
  - `gap` → `NumberOrExpression::Number(px)`;`justify-content` → 手写 match(`flex-start|start|left`→Start,`center`→Center,`flex-end|end|right`→End,`space-between`→SpaceBetween,`space-around|space-evenly`→SpaceAround);`align-items` → `AlignItems::from_css`。
  - padding 四 longhand → 全等 `Padding::Uniform`,否则 `Padding::LtrB([...])`。**实现前先核对数组序**:读 `vendor/jian/crates/jian-core/src/layout/resolve.rs` 中 `Padding::LtrB` 的消费代码确认四元顺序(变体名提示 Left/Top/Right/Bottom),测试按核实结果写死。
  - 尺寸:`width`/`height` px → `SizingBehavior::Number`;`100%` → `Keyword(FillContainer)`;未声明 → None(容器高度由 mapper 上层在 Task 8 决定 hug/fill 默认)。
  - `background-color` → `PenFill::Solid(SolidFillBody { color, explain: None, opacity: None, blend_mode: None })`;`background-image` 含 `linear-gradient(` → 解析 `angle`(`Ndeg`,`to right`→90 等四向)与颜色 stops(均分 offset 当未显式给出)→ `PenFill::LinearGradient`;含 `radial-gradient(` → stops 同法 → `PenFill::RadialGradient`;含 `url(` → `PenFill::Image`(**实现时先读 `vendor/jian/crates/jian-ops-schema/src/style.rs:214` 的 `ImageFillBody` 字段清单**,`url` 用 `ImageSrc::from(url_str)`,其余字段全 None;若无 `Default` 实现则显式列全)。
  - `border-width/-color` → `PenStroke { thickness: StrokeThickness::Uniform(w), align: None, join: None, cap: None, dash_pattern: None, dash_offset: None, fill: Some(vec![solid(color)]) }`;四边不全等 → 取最大宽 `Uniform` + warning(spec:取最粗边)。
  - `border-radius` 1-4 值 → 全等 `CornerRadius::Uniform`,否则 `PerCorner([tl,tr,br,bl])`。
  - `box-shadow` → 逐层解析 `[inset] x y blur spread color` → `PenEffect::Shadow(ShadowBody { inner, offset_x, offset_y, blur, spread, color })`(缺省 spread=0);多层全收进 `effects`。
  - `overflow:hidden` → `clip_content: Some(true)`;`opacity` → base 的 `opacity: Some(NumberOrExpression::Number(v))`;`transform: rotate(Ndeg)` → base `rotation: Some(N)`,其余 transform → warning(warn_once)。
- 节点构造:`FrameNode` 无 `Default`,写一个私有 helper(仿 `op-figma/src/node_build.rs:15` 的 `frame_node`):

```rust
fn frame(base: PenNodeBase, container: ContainerProps, children: Vec<PenNode>) -> PenNode {
    PenNode::Frame(FrameNode {
        base, container,
        children: Some(children),
        image_search_query: None, reusable: None, slot: None, state: None,
        bindings: None, events: None, lifecycle: None, semantics: None,
        gestures: None, route: None, screen: None,
    })
}
```

- [ ] **Step 1: 写失败测试(mapper.rs 底部;直接构 DomElement + 规则字符串,断言产出的 PenNode)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::cascade::parse_stylesheet;
    use jian_ops_schema::node::container::{LayoutMode, JustifyContent, AlignItems, Padding, CornerRadius};
    use jian_ops_schema::node::PenNode;
    use jian_ops_schema::style::{PenFill, PenEffect, StrokeThickness};

    fn map_one(html_el: crate::dom::DomElement, css: &str) -> Option<PenNode> {
        let (rules, _) = parse_stylesheet(css, 1000);
        let opts = crate::HtmlImportOptions::default();
        let mut ctx = MapCtx { opts: &opts, rules: &rules, warnings: Vec::new(), next_id: 0, node_count: 0 };
        map_element(&mut ctx, &[&html_el], None)
    }

    #[test]
    fn flex_row_maps_to_horizontal_frame() {
        let el = crate::dom::DomElement { tag: "div".into(),
            attrs: vec![("style".into(),
                "display:flex;gap:12px;justify-content:space-between;align-items:center;padding:16px".into())],
            children: Vec::new() };
        let Some(PenNode::Frame(f)) = map_one(el, "") else { panic!("expected frame") };
        assert_eq!(f.container.layout, Some(LayoutMode::Horizontal));
        assert!(matches!(f.container.gap, Some(jian_ops_schema::node::base::NumberOrExpression::Number(v)) if v == 12.0));
        assert_eq!(f.container.justify_content, Some(JustifyContent::SpaceBetween));
        assert_eq!(f.container.align_items, Some(AlignItems::Center));
        assert!(matches!(f.container.padding, Some(Padding::Uniform(v)) if v == 16.0));
    }

    #[test]
    fn visual_styles_map_to_fill_stroke_effects() {
        let el = crate::dom::DomElement { tag: "div".into(),
            attrs: vec![("style".into(),
                "background-color:#102030;border:2px solid #ff0000;border-radius:8px;\
                 box-shadow:0 4px 8px rgba(0,0,0,0.25);overflow:hidden".into())],
            children: Vec::new() };
        let Some(PenNode::Frame(f)) = map_one(el, "") else { panic!() };
        let fills = f.container.fill.as_ref().unwrap();
        assert!(matches!(&fills[0], PenFill::Solid(s) if s.color == "#102030"));
        let stroke = f.container.stroke.as_ref().unwrap();
        assert!(matches!(stroke.thickness, StrokeThickness::Uniform(w) if w == 2.0));
        assert!(matches!(f.container.corner_radius, Some(CornerRadius::Uniform(r)) if r == 8.0));
        let effects = f.container.effects.as_ref().unwrap();
        assert!(matches!(&effects[0], PenEffect::Shadow(s)
            if s.offset_y == 4.0 && s.blur == 8.0 && s.color == "#00000040"));
        assert_eq!(f.container.clip_content, Some(true));
    }

    #[test]
    fn linear_gradient_fill() {
        let el = crate::dom::DomElement { tag: "div".into(),
            attrs: vec![("style".into(), "background:linear-gradient(90deg,#000000,#ffffff)".into())],
            children: Vec::new() };
        let Some(PenNode::Frame(f)) = map_one(el, "") else { panic!() };
        let fills = f.container.fill.as_ref().unwrap();
        let PenFill::LinearGradient(g) = &fills[0] else { panic!("expected gradient") };
        assert_eq!(g.angle, Some(90.0));
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[1].color, "#ffffff");
        assert_eq!(g.stops[1].offset, 1.0);
    }

    #[test]
    fn display_none_is_skipped() {
        let el = crate::dom::DomElement { tag: "div".into(),
            attrs: vec![("style".into(), "display:none".into())], children: Vec::new() };
        assert!(map_one(el, "").is_none());
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html mapper`。
- [ ] **Step 3: 实现**(`map_element` 本任务只处理:display 判定 → 递归 children(`DomNode::Text` 先忽略,Task 9 接管)→ `container_props_from` → `frame()`;`generate_id` 自增;`node_count` 每产出一个节点 +1)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html mapper`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): map containers and visual styles to frame nodes"
```

---

### Task 8: mapper 布局启发式 — margin→gap 众数、尺寸默认、绝对定位、降级告警

**Files:**
- Modify: `crates/op-html/src/mapper.rs`(若接近 800 行,拆出 `crates/op-html/src/layout_heuristics.rs`)

**Interfaces:**
- Consumes: Task 7 的 `MapCtx/map_element/container_props_from`
- Produces(mapper 内部函数,签名固定供测试):
  - `pub fn infer_gap_from_margins(child_styles: &[&ComputedStyle], ctx_font: f64) -> (Option<f64>, bool)` — 返回 (众数 gap, 是否有偏差);相邻对 gap = min(?) 不,取 `margin-bottom(prev) + margin-top(next)`(简化折叠);众数 = 出现最多的值(并列取较小);偏差存在时调用方记 warning
  - 尺寸默认规则(实现在 `map_element` 收尾处):块级元素(父为 Vertical 且自身未声明 width)→ `width: Keyword(FillContainer)`;所有容器未声明 height → `height: Keyword(FitContent)`;`flex-grow` > 0 → 主轴尺寸 `Keyword(FillContainer)`(父 Horizontal→width,父 Vertical→height)
  - 绝对定位:`position:absolute|fixed` 且 left/top 为 px → base `x/y = Some(px)`(spec 已核实 `jian-core/src/layout/resolve.rs:141`:带显式 x/y 的子节点按 Absolute 处理);left/top 为 `%` → 按 viewport 近似换算 + warning
  - 降级 warning(全部 `warn_once`):`display:grid`(近似 Vertical)、`flex-wrap:wrap`(忽略换行)、`float`(忽略)

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn gap_mode_from_margins() {
    // three children with uniform 16px collapsed margins, one odd 24px pair
    let mk = |mt: &str, mb: &str| {
        let mut c = ComputedStyle { props: Default::default(), font_size: 16.0 };
        c.props.insert("margin-top".into(), mt.into());
        c.props.insert("margin-bottom".into(), mb.into());
        c
    };
    let s1 = mk("0", "8px"); let s2 = mk("8px", "8px");
    let s3 = mk("8px", "16px"); let s4 = mk("8px", "0");
    let styles: Vec<&ComputedStyle> = vec![&s1, &s2, &s3, &s4];
    let (gap, deviated) = infer_gap_from_margins(&styles, 16.0);
    assert_eq!(gap, Some(16.0)); // pairs: 16,16,24 → mode 16
    assert!(deviated);
}

#[test]
fn block_child_defaults_to_fill_width_and_frames_hug_height() {
    let parent = crate::dom::DomElement { tag: "div".into(), attrs: vec![],
        children: vec![crate::dom::DomNode::Element(crate::dom::DomElement {
            tag: "div".into(), attrs: vec![], children: Vec::new() })] };
    let Some(PenNode::Frame(f)) = map_one(parent, "") else { panic!() };
    let PenNode::Frame(child) = &f.children.as_ref().unwrap()[0] else { panic!() };
    use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
    assert_eq!(child.container.width, Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)));
    assert_eq!(child.container.height, Some(SizingBehavior::Keyword(SizingKeyword::FitContent)));
}

#[test]
fn absolute_positioning_lands_on_base_xy() {
    let el = crate::dom::DomElement { tag: "div".into(),
        attrs: vec![("style".into(), "position:absolute;left:24px;top:48px".into())],
        children: Vec::new() };
    let Some(PenNode::Frame(f)) = map_one(el, "") else { panic!() };
    assert_eq!(f.base.x, Some(24.0));
    assert_eq!(f.base.y, Some(48.0));
}

#[test]
fn grid_degrades_with_single_warning() {
    let (rules, _) = crate::css::cascade::parse_stylesheet("", 1000);
    let opts = crate::HtmlImportOptions::default();
    let mut ctx = MapCtx { opts: &opts, rules: &rules, warnings: Vec::new(), next_id: 0, node_count: 0 };
    for _ in 0..2 {
        let el = crate::dom::DomElement { tag: "div".into(),
            attrs: vec![("style".into(), "display:grid".into())], children: Vec::new() };
        map_element(&mut ctx, &[&el], None);
    }
    assert_eq!(ctx.warnings.iter().filter(|w| w.contains("grid")).count(), 1);
}
```

(`map_one` 复用 Task 7 测试模块里的 helper。)

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html mapper`。
- [ ] **Step 3: 实现**(注意 `infer_gap_from_margins` 众数并列取较小值;检查 mapper.rs 行数,>800 则把本任务函数移入 `layout_heuristics.rs` 并在 lib.rs 声明)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html mapper`。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): layout heuristics for gap inference, sizing defaults and absolute children"
```

---

### Task 9: text.rs — 行内内容合并为 TextNode

**Files:**
- Create: `crates/op-html/src/text.rs`
- Modify: `crates/op-html/src/lib.rs`(加 `pub mod text;`);`mapper.rs` 的 `map_element` 接入

**Interfaces:**
- Consumes: `DomNode/DomElement`、`ComputedStyle`
- Produces:
  - `pub fn is_inline_tag(tag: &str) -> bool` — `a b strong i em u s del strike span code small sub sup label br mark`
  - `pub fn build_text_node(ctx: &mut MapCtx, run: &[&DomNode], block_style: &ComputedStyle) -> Option<PenNode>` — 把一段连续行内内容(元素+文本)合并为一个 `PenNode::Text`;全段无样式差异 → `TextContent::Plain`,否则 `TextContent::Styled(Vec<StyledTextSegment>)`
  - mapper 集成规则:元素 children 按"连续行内 run / 块级元素"切段;每个行内 run 产出一个 Text 节点,块级子元素递归 `map_element`;空白折叠(`split_whitespace().join(" ")`),纯空白 run 丢弃;`<br>` 在 run 内转换成 `\n`
- 段样式继承:递归下行时携带 `SegStyle { weight, style, underline, strike, fill, href, font_size, font_family }`,遇 `b/strong`(weight=700)、`i/em`(italic)、`u`(underline)、`s/del/strike`(strikethrough)、`a`(href + 依该元素 computed color/下划线)、行内元素自身 computed(inline style/class 命中)逐层覆盖
- TextNode 顶层属性取 `block_style`:`font_family/font_size(f64)/font_weight(FontWeight::Number)/font_style/letter_spacing(px)/line_height`(无单位倍数直存;px 值 ÷ font_size 归一为倍数)/`text_align`;`color` → `fill: Some(vec![PenFill::Solid])`
- 构造:`TextNode` 无 `Default`,显式列全字段(`state/bindings/events/lifecycle/semantics/gestures/route` 全 None,`width/height/text_align_vertical/text_growth/underline/strikethrough/effects` 按需/None)

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::node::text::TextContent;
    use jian_ops_schema::node::PenNode;

    fn text_of(html: &str, css: &str) -> Vec<PenNode> {
        // parse_dom + map via mapper on a single wrapping div, return its children
        let dom = crate::dom::parse_dom(html);
        let (mut rules, _) = crate::css::cascade::parse_stylesheet(crate::css::cascade::UA_STYLESHEET, 0);
        let (author, _) = crate::css::cascade::parse_stylesheet(css, 1000);
        rules.extend(author);
        let opts = crate::HtmlImportOptions::default();
        let mut ctx = crate::mapper::MapCtx { opts: &opts, rules: &rules,
            warnings: Vec::new(), next_id: 0, node_count: 0 };
        let crate::dom::DomNode::Element(root) = &dom.body[0] else { panic!() };
        let Some(PenNode::Frame(f)) = crate::mapper::map_element(&mut ctx, &[root], None) else { panic!() };
        f.children.unwrap_or_default()
    }

    #[test]
    fn plain_paragraph_merges_to_single_plain_text() {
        let kids = text_of("<div><p>hello   world</p></div>", "");
        let PenNode::Frame(p) = &kids[0] else { panic!("p should be a frame") };
        let PenNode::Text(t) = &p.children.as_ref().unwrap()[0] else { panic!("expected text") };
        assert!(matches!(&t.content, TextContent::Plain(s) if s == "hello world"));
    }

    #[test]
    fn bold_link_run_becomes_styled_segments() {
        let kids = text_of("<div><p>see <b>bold</b> and <a href=\"https://x.dev\">link</a></p></div>", "");
        let PenNode::Frame(p) = &kids[0] else { panic!() };
        let PenNode::Text(t) = &p.children.as_ref().unwrap()[0] else { panic!() };
        let TextContent::Styled(segs) = &t.content else { panic!("expected styled") };
        assert_eq!(segs.len(), 4); // "see ", "bold", " and ", "link"
        assert_eq!(segs[1].font_weight, Some(700));
        assert_eq!(segs[3].href.as_deref(), Some("https://x.dev"));
        assert_eq!(segs[3].underline, Some(true)); // ua: a { text-decoration: underline }
        assert_eq!(segs[3].fill.as_deref(), Some("#0066cc"));
    }

    #[test]
    fn block_font_props_come_from_computed_style() {
        let kids = text_of("<div><h1>Title</h1></div>", "");
        let PenNode::Frame(h1) = &kids[0] else { panic!() };
        let PenNode::Text(t) = &h1.children.as_ref().unwrap()[0] else { panic!() };
        assert_eq!(t.font_size, Some(32.0)); // ua h1
        use jian_ops_schema::node::text::FontWeight;
        assert!(matches!(t.font_weight, Some(FontWeight::Number(700))));
    }

    #[test]
    fn mixed_inline_and_block_children_split_into_runs() {
        let kids = text_of("<div>intro <b>x</b><section></section>tail</div>", "");
        assert_eq!(kids.len(), 3); // Text("intro x"), Frame(section), Text("tail")
        assert!(matches!(kids[0], PenNode::Text(_)));
        assert!(matches!(kids[1], PenNode::Frame(_)));
        assert!(matches!(kids[2], PenNode::Text(_)));
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html text`。
- [ ] **Step 3: 实现**(注意 `<p>` 等纯文本块自身仍是 Frame(承载 margin/padding/background),其行内内容成为它的 Text 子节点;只有一个 Plain 段且无覆盖时用 Plain)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html text` 且 `cargo test -p op-html` 全绿。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): merge inline runs into styled text nodes"
```

---

### Task 10: 特殊元素 — img / 内联 svg / 表单控件 / hr

**Files:**
- Create: `crates/op-html/src/special.rs`
- Modify: `crates/op-html/src/lib.rs`(加 `pub mod special;`);`mapper.rs` 的 `map_element` 前置分发

**Interfaces:**
- Consumes: `DomElement/ComputedStyle/MapCtx`
- Produces: `pub fn map_special(ctx: &mut MapCtx, el: &DomElement, style: &ComputedStyle) -> Option<Option<PenNode>>` — 外层 `None` = 非特殊元素(走通用容器/文本路径);`Some(None)` = 特殊但应跳过(如 iframe/video 的占位在内,返回占位 Frame 则是 `Some(Some(...))`)
- 映射(spec"特殊元素"节):
  - `<img>` → `PenNode::Image(ImageNode)`:`src` 原样 `ImageSrc::from`(URL 或 data URL;抓取内嵌属子项目 2);`object-fit` → `ImageFitMode`(cover→Crop,contain→Fit,fill→Fill,默认 None);width/height 属性或 computed px → `SizingBehavior::Number`;`border-radius` → `corner_radius`。`ImageNode` 无 `Default`,显式列全字段(photo-filter 系数/prompt/行为槽全 None)。
  - 内联 `<svg>` → 序列化该元素(标签+属性+子树重建源码)→ `PenNode::Image`,`src = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(serialized))` + warning `"inline <svg> imported as image placeholder"`(warn_once)。
  - 表单控件(全部 `Default` 可用,`..Default::default()`):
    - `<input type=text|email|password|search|url|tel|无type>` → `TextInputNode { base, placeholder: attr("placeholder"), value: attr("value"), ..Default::default() }`
    - `<textarea>` → `TextAreaNode`(placeholder/value=文本内容)
    - `<select>` → `SelectNode`,`options` = `<option>` 子元素 → `SelectOption { value: attr("value") 或文本, label: 文本 }`,`value` = 带 `selected` 的 option
    - `<input type=checkbox>` → `CheckboxNode { checked: Some(BoolOrExpression::Bool(has_attr("checked"))), label: None, .. }`(相邻 label 合并不做,YAGNI)
    - 同一父元素下同 `name` 的 `<input type=radio>` 组 → 一个 `RadioGroupNode`(options 取各 radio 的 value;mapper 层聚合)
    - `<input type=range>` → `SliderNode { min/max/step/value: 各 attr 解析 f64, .. }`
    - `<progress>` → `ProgressNode { value: attr, max: attr, .. }`
    - `<button>` → 通用容器路径但 base `role: Some("button".into())`(在 mapper 里处理,不在 special 里)
  - `<hr>` → `RectangleNode`(height Number(1),width FillContainer,fill `#e0e0e0`;用 Task 7 的构造 helper 补一个 `rectangle()`)
  - `<iframe>/<video>/<canvas>` → 占位 Frame(灰底 `#f0f0f0`,尺寸取 computed/attr)+ warning(warn_once,各自计一条)
- 控件的 width/height/fill/stroke/corner_radius 同容器规则从 computed 带入。

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use jian_ops_schema::node::PenNode;

    fn el_with(tag: &str, attrs: Vec<(&str, &str)>) -> crate::dom::DomElement {
        crate::dom::DomElement { tag: tag.into(),
            attrs: attrs.into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            children: Vec::new() }
    }
    fn map_it(el: crate::dom::DomElement) -> Option<PenNode> {
        let (rules, _) = crate::css::cascade::parse_stylesheet("", 0);
        let opts = crate::HtmlImportOptions::default();
        let mut ctx = crate::mapper::MapCtx { opts: &opts, rules: &rules,
            warnings: Vec::new(), next_id: 0, node_count: 0 };
        crate::mapper::map_element(&mut ctx, &[&el], None)
    }

    #[test]
    fn img_maps_to_image_node() {
        let n = map_it(el_with("img", vec![("src", "https://x.dev/a.png"), ("width", "120"), ("height", "80")]));
        let Some(PenNode::Image(img)) = n else { panic!("expected image") };
        assert_eq!(img.src.as_str(), "https://x.dev/a.png");
        use jian_ops_schema::sizing::SizingBehavior;
        assert!(matches!(img.width, Some(SizingBehavior::Number(v)) if v == 120.0));
    }

    #[test]
    fn inline_svg_becomes_data_url_image() {
        let mut svg = el_with("svg", vec![("viewBox", "0 0 10 10")]);
        svg.children.push(crate::dom::DomNode::Element(el_with("rect", vec![("width", "10")])));
        let Some(PenNode::Image(img)) = map_it(svg) else { panic!() };
        assert!(img.src.as_str().starts_with("data:image/svg+xml;base64,"));
    }

    #[test]
    fn form_controls_map_to_widget_nodes() {
        assert!(matches!(map_it(el_with("input", vec![("type", "text"), ("placeholder", "Name")])),
            Some(PenNode::TextInput(t)) if t.placeholder.as_deref() == Some("Name")));
        assert!(matches!(map_it(el_with("input", vec![("type", "checkbox"), ("checked", "")])),
            Some(PenNode::Checkbox(_))));
        assert!(matches!(map_it(el_with("input", vec![("type", "range"), ("min", "0"), ("max", "10")])),
            Some(PenNode::Slider(s)) if s.max == Some(10.0)));
        assert!(matches!(map_it(el_with("progress", vec![("value", "3"), ("max", "10")])),
            Some(PenNode::Progress(_))));
    }

    #[test]
    fn select_collects_options() {
        let mut sel = el_with("select", vec![]);
        let mut opt = el_with("option", vec![("value", "a"), ("selected", "")]);
        opt.children.push(crate::dom::DomNode::Text("Alpha".into()));
        sel.children.push(crate::dom::DomNode::Element(opt));
        let Some(PenNode::Select(s)) = map_it(sel) else { panic!() };
        assert_eq!(s.value.as_deref(), Some("a"));
        let opts = s.options.as_ref().unwrap();
        assert_eq!(opts[0].label, "Alpha");
    }

    #[test]
    fn button_is_frame_with_role() {
        let mut b = el_with("button", vec![]);
        b.children.push(crate::dom::DomNode::Text("Go".into()));
        let Some(PenNode::Frame(f)) = map_it(b) else { panic!() };
        assert_eq!(f.base.role.as_deref(), Some("button"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html special`。
- [ ] **Step 3: 实现**(svg 序列化:递归重建 `<tag attr="v">…</tag>`,文本原样;radio 聚合在 mapper 的 children 循环里做:遇到 radio 时收集同父同 name 的兄弟,产出一个 RadioGroup 并跳过其余)。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p op-html`,全绿。
- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): map img, inline svg, form controls and hr to schema nodes"
```

---

### Task 11: lib.rs 管线编排 — 根 Frame、防御上限、端到端测试、wasm 检查

**Files:**
- Modify: `crates/op-html/src/lib.rs`(替换 Task 1 的占位实现)
- Create: `crates/op-html/src/e2e_tests.rs`(`#[cfg(test)] mod e2e_tests;` 在 lib.rs 声明,仿 `op-figma/src/binary_e2e_tests.rs` 的同级测试文件模式)

**Interfaces:**
- Consumes: 全部前置任务
- Produces: 最终的 `import_html`(签名不变):
  1. `source.len() > 10 * 1024 * 1024` → 截断到 10MB(按 char 边界)+ warning。
  2. `dom::parse_dom` → body 为空(无元素且无非空白文本)→ `no importable content` warning + 空结果。
  3. 规则集 = `parse_stylesheet(UA_STYLESHEET, 0)` + 各 `style_blocks` 依次 `parse_stylesheet(block, 1000 + i * 10_000)`;stylesheet warnings 并入结果。
  4. body 元素的 computed style(把 body 当元素算,或无 body 包裹时用默认)→ 根 Frame:`name` = `opts.document_name` > `dom.title` > `"HTML Import"`;`width: Number(opts.viewport_width)`;`height: Keyword(FitContent)`;`layout: Vertical`;`fill` = body `background-color` 或 `#ffffff`;`padding` = body padding;body children 逐个走 `map_element`/文本 run。
  5. 映射循环中 `ctx.node_count >= 20_000` → 停止 + warning `"node limit reached (20000), remaining content dropped"`。
  6. 返回 `HtmlImportResult { nodes: vec![root_frame], warnings }`(root 恒为单个 Frame)。

- [ ] **Step 1: 写失败的端到端测试(e2e_tests.rs)**

```rust
use crate::{import_html, HtmlImportOptions};
use jian_ops_schema::node::PenNode;
use jian_ops_schema::node::container::LayoutMode;
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};

const LANDING: &str = r#"<html><head><title>Acme</title><style>
  .hero { display:flex; flex-direction:column; align-items:center; gap:24px;
          padding:64px; background:linear-gradient(180deg,#0b1220,#1a2740); }
  .hero h1 { color:#ffffff; margin:0 }
  .cta { background-color:#3b82f6; color:#ffffff; padding:12px 24px; border-radius:8px }
  .row { display:flex; gap:16px }
</style></head><body>
  <section class="hero">
    <h1>Build faster</h1>
    <p style="color:#94a3b8">Ship <b>beautiful</b> designs</p>
    <div class="row">
      <button class="cta">Start</button>
      <input type="text" placeholder="Email"/>
    </div>
  </section>
</body></html>"#;

#[test]
fn landing_page_imports_as_editable_tree() {
    let r = import_html(LANDING, &HtmlImportOptions::default());
    assert_eq!(r.nodes.len(), 1);
    let PenNode::Frame(root) = &r.nodes[0] else { panic!("root must be frame") };
    assert_eq!(root.base.name.as_deref(), Some("Acme"));
    assert_eq!(root.container.width, Some(SizingBehavior::Number(1440.0)));
    assert_eq!(root.container.height, Some(SizingBehavior::Keyword(SizingKeyword::FitContent)));
    let PenNode::Frame(hero) = &root.children.as_ref().unwrap()[0] else { panic!() };
    assert_eq!(hero.container.layout, Some(LayoutMode::Vertical));
    let kids = hero.children.as_ref().unwrap();
    // h1 frame → text child; p frame; row frame with button + input
    assert!(kids.len() >= 3);
    let PenNode::Frame(row) = kids.last().unwrap() else { panic!("row") };
    let row_kids = row.children.as_ref().unwrap();
    assert!(matches!(&row_kids[0], PenNode::Frame(b) if b.base.role.as_deref() == Some("button")));
    assert!(matches!(&row_kids[1], PenNode::TextInput(_)));
}

#[test]
fn node_limit_truncates_with_warning() {
    let mut html = String::from("<div>");
    for _ in 0..25_000 { html.push_str("<p>x</p>"); }
    html.push_str("</div>");
    let r = import_html(&html, &HtmlImportOptions::default());
    assert!(r.warnings.iter().any(|w| w.contains("node limit")));
}

#[test]
fn document_name_option_overrides_title() {
    let opts = HtmlImportOptions { document_name: Some("Custom".into()), ..Default::default() };
    let r = import_html("<html><head><title>T</title></head><body><p>x</p></body></html>", &opts);
    let PenNode::Frame(root) = &r.nodes[0] else { panic!() };
    assert_eq!(root.base.name.as_deref(), Some("Custom"));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-html e2e`(Task 1 的占位实现返回空 nodes)。
- [ ] **Step 3: 实现编排**(Task 1 占位的 `"importer pipeline not yet implemented"` warning 删除;同步更新 Task 1 的第二条测试若受影响——空输入行为不变)。
- [ ] **Step 4: 全量验证**

Run: `cargo test -p op-html`
Expected: 全绿。
Run: `cargo check --target wasm32-unknown-unknown -p op-html`
Expected: 通过(wasm32-clean 达成)。
Run: `cargo clippy -p op-html --all-targets -- -D warnings`
Expected: 无警告。

- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-html
git add crates/op-html/src
git commit --no-verify -m "feat(html): wire full import pipeline with root frame and defense limits"
```

---

### Task 12: MCP `import_html` 工具

**Files:**
- Modify: `crates/op-mcp/Cargo.toml`(加 `op-html = { path = "../op-html" }`)
- Create: `crates/op-mcp/src/import_html_tool.rs`(不进 `write_tools.rs`——该文件已 808 行触顶)
- Create: `crates/op-mcp/src/import_html_tool_tests.rs`(仿 `batch_design_tests.rs` 同级测试文件模式)
- Modify: `crates/op-mcp/src/lib.rs`(声明 `pub mod import_html_tool;` 与 `#[cfg(test)] mod import_html_tool_tests;`——先 grep 现有 `mod batch_design_tests` 的声明方式照抄)
- Modify: `crates/op-mcp/src/write_tools.rs:501`(`fn parse_opt_i32` → `pub(crate) fn parse_opt_i32`,仅此一行)
- Modify: `crates/op-host-services/src/mcp_serve.rs`(注册 + import)
- Modify: `crates/op-host-services/src/mcp_serve/schemas.rs`(schema 常量数组加一项)

**Interfaces:**
- Consumes: `op_html::{import_html, HtmlImportOptions}`;`op-mcp` 现有 `McpTool` trait、`ToolOutcome`、`ToolErrorCode`;write_tools 的 helper **可见性需调整**:`root_or_node_id` 现为 `pub(super)`(write_tools.rs:545,兄弟模块经 `super::write_tools::` 可用),`parse_opt_i32` 现为私有(write_tools.rs:501)→ 改为 `pub(crate) fn parse_opt_i32`(一行改动,计入本任务 Files)
- Produces:
  - `pub struct ImportHtml; impl McpTool for ImportHtml`(`name() = "import_html"`)
  - `pub fn import_html_snapshot() -> ImportHtml`
- 行为(镜像 `write_tools.rs:727` 的 `ImportSvg`):
  - `html` 参数,缺省回退 `htmlPath`/`html_path`(`fs::read_to_string`);二者皆无 → `MissingArgument`;空 → `InvalidArgument`。
  - `x`/`y` 经 `parse_opt_i32`(默认 0);`parent`/`parent_id`/`target_parent_id` → `root_or_node_id`(默认 `NodeId::NONE`);`pageId`/`page_id`/`page` → `Option<String>`。
  - 调 `op_html::import_html(&html, &HtmlImportOptions::default())`;`nodes` 为空 → `ToolOutcome::Err(ToolErrorCode::InvalidArgument, "no importable content: ..."(带首条 warning))`。
  - x/y ≠ 0 时写到根节点:`if let PenNode::Frame(f) = &mut nodes[0] { f.base.x = Some(x as f64); f.base.y = Some(y as f64); }`。
  - out:`wrote=true`、`nodeCount=<n>`(递归计数)、warnings 非空时 `warnings=<join("\n")>`。
  - 返回 `ToolOutcome::OkWithCommand(out, EditorCommand::InsertSubtree { nodes, parent_id: target_parent, page_id })`(变体字段见 `op-editor-core/src/command.rs:242`)。

- [ ] **Step 1: 写失败测试(import_html_tool_tests.rs)**

```rust
use std::collections::BTreeMap;

use super::import_html_tool::{import_html_snapshot, ImportHtml};
use super::{EditorCommand, McpTool, ToolOutcome};
// 同级测试文件模式:batch_design_tests.rs:9 就是 `use super::{..., EditorCommand, McpTool, ...}`,
// EditorCommand 经 op-mcp lib.rs:141 的 `pub use op_editor_core::{...}` 再导出。

#[test]
fn import_html_returns_insert_subtree_command() {
    let tool: ImportHtml = import_html_snapshot();
    let mut args = BTreeMap::new();
    args.insert("html".to_string(), "<div style=\"display:flex\"><p>hi</p></div>".to_string());
    args.insert("x".to_string(), "100".to_string());
    let out = tool.call(&args);
    let ToolOutcome::OkWithCommand(map, EditorCommand::InsertSubtree { nodes, page_id, .. }) = out
        else { panic!("expected OkWithCommand(InsertSubtree)") };
    assert_eq!(map.get("wrote").map(String::as_str), Some("true"));
    assert_eq!(nodes.len(), 1);
    assert!(page_id.is_none());
}

#[test]
fn missing_html_is_typed_error() {
    let tool = import_html_snapshot();
    let out = tool.call(&BTreeMap::new());
    assert!(matches!(out, ToolOutcome::Err(..)));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-mcp import_html`。
- [ ] **Step 3: 实现工具 + 注册**

`mcp_serve.rs`(35 行附近现有 write_tools import 处加):

```rust
use op_mcp::import_html_tool::import_html_snapshot;
```

`mcp_serve.rs:589`(`import_svg` 注册行之后加):

```rust
register_tool!("import_html", import_html_snapshot());
```

`schemas.rs`(122 行 `import_svg` 条目之后加一项):

```rust
r#"{"name":"import_html","description":"Parse an HTML document or snippet (CSS-subset cascade: inline style, <style> blocks, tag/class/id descendant selectors) and insert the resulting editable auto-layout nodes on the active or requested page. Accepts inline html or htmlPath.","inputSchema":{"type":"object","properties":{"filePath":{"type":"string","description":"Optional target .op file path; omit to use the server document"},"html":{"type":"string","description":"HTML document or fragment text"},"htmlPath":{"type":"string","description":"local HTML file path"},"x":{"type":"string","description":"i32 doc-px x offset (default 0)"},"y":{"type":"string","description":"i32 doc-px y offset (default 0)"},"parent":{"type":"string","description":"optional parent node id; empty/0/root omitted = page root"},"pageId":{"type":"string","description":"optional target page id or legacy page index; omitted = active page"}}}}"#,
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p op-mcp import_html` → 2 passed;`cargo check -p op-host-services` → 通过。

- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-mcp -p op-host-services
git add crates/op-mcp crates/op-host-services
git commit --no-verify -m "feat(mcp): import_html tool inserting op-html parsed subtrees"
```

---

### Task 13: CLI `op import:html`

**Files:**
- Modify: `crates/op-cli/src/main.rs`(dispatch 分支 + `map_import_html` 函数 + help 文本)
- Modify: `crates/op-cli/src/tests.rs`(单测)
- **不要触碰** `crates/op-cli/Cargo.toml` 与 `figma_cli.rs` 的现有未提交改动;`import:html` 走 tool_call 路径,不需要新依赖。

**Interfaces:**
- Consumes: main.rs 现有 `required_pos`/`pair`/`flag_value`/`push_file_path`/`tool_call` helper(与 `map_import_svg`(main.rs:678)完全同型)
- Produces: 子命令 `op import:html <file.html> [--x N] [--y N] [--parent P] [--page PAGE]` → `tool_call("import_html", pairs)`,`htmlPath` 传文件路径

- [ ] **Step 1: 写失败测试(tests.rs;样式与 tests.rs:200 `parse_args_maps_ts_get_id_alias_to_rust_tool` 完全同构)**

```rust
#[test]
fn import_html_maps_to_tool_call() {
    let args = vec![
        "import:html".to_string(),
        "page.html".to_string(),
        "--x".to_string(),
        "10".to_string(),
        "--page".to_string(),
        "p1".to_string(),
    ];
    let p = parse_args(&args).expect("parse");
    assert_eq!(
        p.command,
        Command::ToolCall {
            tool: "import_html".to_string(),
            args: vec![
                ("htmlPath".to_string(), "page.html".to_string()),
                ("x".to_string(), "10".to_string()),
                ("pageId".to_string(), "p1".to_string()),
            ],
        }
    );
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p op-cli import_html`。
- [ ] **Step 3: 实现**

`map_import_svg`(main.rs:678)之后加:

```rust
fn map_import_html(positionals: &[String], flags: &Flags) -> Result<Command, String> {
    let path = required_pos(
        positionals,
        1,
        "Usage: op import:html <file.html> [--x N] [--y N] [--parent P] [--page PAGE]",
    )?;
    let mut pairs = vec![pair("htmlPath", path)];
    if let Some(x) = flag_value(flags, "x") {
        pairs.push(pair("x", x));
    }
    if let Some(y) = flag_value(flags, "y") {
        pairs.push(pair("y", y));
    }
    if let Some(parent) = flag_value(flags, "parent") {
        pairs.push(pair("parent", parent));
    }
    if let Some(page) = flag_value(flags, "page") {
        pairs.push(pair("pageId", page));
    }
    push_file_path(&mut pairs, flags);
    tool_call("import_html", pairs)
}
```

dispatch match(main.rs:362 `"import:svg"` 分支旁)加 `"import:html" => map_import_html(&positionals, &flags),`(具体 arm 形式照抄 import:svg 行)。然后 `grep -n "import:svg" crates/op-cli/src/main.rs` 找到 help/usage 文本中的每一处提及,同步补一行 `import:html` 说明(文案:`import:html <file.html>   Import an HTML file onto the active page`)。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p op-cli` → 全绿;`cargo build -p op-cli` → 通过。
手动冒烟(可选,需要跑着的 desktop/serve-web MCP):`./target/debug/op import:html /tmp/t.html`。

- [ ] **Step 5: 提交**

```bash
cargo fmt -p op-cli
git add crates/op-cli/src/main.rs crates/op-cli/src/tests.rs
git commit --no-verify -m "feat(cli): import:html subcommand mapping to the import_html mcp tool"
```

---

## 收尾验证(全部任务完成后)

- [ ] `cargo test --workspace`(全绿;若 legacy 失败与本计划无关,记录并上报)
- [ ] `cargo clippy -p op-html -p op-mcp -p op-cli --all-targets -- -D warnings`
- [ ] `cargo check --target wasm32-unknown-unknown -p op-html`
- [ ] 对照 spec"子项目 1"逐节确认:映射规则每条有实现与测试;MCP/CLI 入口行为与 `import_svg` 同构;warnings 通道贯通(op-html → MCP out.warnings / CLI JSON)
- [ ] op-smoke 冒烟(spec"测试"节要求):`grep -rn "import_svg" crates/op-smoke/src/` 找到现有 MCP 工具冒烟样例,镜像加一条 `import_html`(inline html → 断言 `wrote=true` 与 nodeCount≥1);若 op-smoke 无任何工具级冒烟先例,记录该事实并跳过(不自创冒烟框架)



