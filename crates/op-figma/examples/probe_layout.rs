//! Preserve-mode layout probe for a real `.fig` file.
//!
//! Prints converted parent-local geometry beside the absolute geometry
//! consumed by the renderer, and can raster the matched subtree through the
//! same scene painter used by the desktop canvas.
//!
//! Usage:
//! `cargo run -p op-figma --example probe_layout -- <file.fig> --page <name> --target <name> [--ancestor N] [--depth N] [--shot out.png]`

use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::sizing::SizingBehavior;
use op_editor_ui::layout_scene::{SceneNode, ScenePage};
use op_figma::{parse_fig_binary, FigLayoutMode};
use op_host_services::export::{export_node_raster, RasterFormat};

#[derive(Debug)]
struct Options {
    path: String,
    page: String,
    target: Option<String>,
    path_contains: Option<String>,
    size: Option<(f64, f64)>,
    ancestor: usize,
    depth: usize,
    limit: usize,
    shot: Option<String>,
}

fn main() {
    let options = options();
    let bytes = std::fs::read(&options.path).expect("read fig");
    let import = parse_fig_binary(&bytes, "tesla-layout-probe", FigLayoutMode::Preserve)
        .expect("parse Preserve import");
    println!("warnings={}", import.warnings.len());

    let pages = import.document.pages.as_deref().unwrap_or(&[]);
    let page_index = pages
        .iter()
        .position(|page| page.name == options.page)
        .unwrap_or_else(|| panic!("page {:?} not found", options.page));
    let page = &pages[page_index];

    let mut state = op_editor_core::EditorState::from_document(import.document.clone());
    state.ui.active_page_index = page_index;
    state.editor_ui.preserve_authored_geometry = true;
    let scene = op_pen_loader::editor_state_to_layout_scene(&state);
    let scene_page = scene.active_page().expect("active scene page");

    let mut stack = Vec::new();
    let mut matches = Vec::new();
    for root in &page.children {
        find_matches(root, &options, &mut stack, &mut matches);
    }
    println!("matches={}", matches.len());
    for (match_index, path) in matches.iter().enumerate() {
        let context_index = path.len().saturating_sub(1 + options.ancestor);
        let context = path[context_index];
        let parent_origin = path[..context_index]
            .iter()
            .fold((0.0, 0.0), |(x, y), node| {
                let base = base_of(node);
                (x + base.x.unwrap_or(0.0), y + base.y.unwrap_or(0.0))
            });
        let names = path
            .iter()
            .map(|node| base_of(node).name.as_deref().unwrap_or("(unnamed)"))
            .collect::<Vec<_>>()
            .join(" / ");
        println!("\n== CONVERTED MATCH {} ==", match_index + 1);
        println!("path: {names}");
        dump_node(context, parent_origin, scene_page, 0, options.depth);
    }

    if let Some(target) = options.shot.as_deref() {
        let path = matches.first().expect("--shot requires at least one match");
        let context_index = path.len().saturating_sub(1 + options.ancestor);
        let id = &base_of(path[context_index]).id;
        let target = std::path::Path::new(target);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("create shot directory");
        }
        export_node_raster(&scene, id, target, RasterFormat::Png, 2.0).expect("render shot");
        println!("shot={}", target.display());
    }
}

fn options() -> Options {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let path = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .cloned()
        .expect("probe_layout <file.fig> --page <name> (--target <name> | --size WxH)");
    let value = |flag: &str| {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
    };
    let size = value("--size")
        .and_then(|value| {
            value
                .split_once('x')
                .map(|(w, h)| (w.to_string(), h.to_string()))
        })
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
    Options {
        path,
        page: value("--page").unwrap_or_else(|| "特斯拉专修首页 v1.0".into()),
        target: value("--target"),
        path_contains: value("--path-contains"),
        size,
        ancestor: value("--ancestor")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        depth: value("--depth")
            .and_then(|value| value.parse().ok())
            .unwrap_or(3),
        limit: value("--limit")
            .and_then(|value| value.parse().ok())
            .unwrap_or(10),
        shot: value("--shot"),
    }
}

fn find_matches<'a>(
    node: &'a PenNode,
    options: &Options,
    stack: &mut Vec<&'a PenNode>,
    matches: &mut Vec<Vec<&'a PenNode>>,
) {
    if matches.len() >= options.limit {
        return;
    }
    stack.push(node);
    let base = base_of(node);
    let name_matches = options
        .target
        .as_ref()
        .is_some_and(|target| base.name.as_deref() == Some(target));
    let size_matches = options.size.is_some_and(|(target_w, target_h)| {
        let (w, h) = numeric_size(node);
        (w - target_w).abs() < 0.01 && (h - target_h).abs() < 0.01
    });
    let path_matches = options.path_contains.as_ref().is_none_or(|needle| {
        stack.iter().any(|entry| {
            base_of(entry)
                .name
                .as_deref()
                .is_some_and(|name| name.contains(needle))
        })
    });
    if (name_matches || size_matches) && path_matches {
        matches.push(stack.clone());
    }
    for child in children_of(node) {
        find_matches(child, options, stack, matches);
    }
    stack.pop();
}

fn dump_node(
    node: &PenNode,
    parent_origin: (f64, f64),
    scene_page: &ScenePage,
    depth: usize,
    max_depth: usize,
) {
    let base = base_of(node);
    let local_x = base.x.unwrap_or(0.0);
    let local_y = base.y.unwrap_or(0.0);
    let abs_x = parent_origin.0 + local_x;
    let abs_y = parent_origin.1 + local_y;
    let (w, h) = numeric_size(node);
    let rendered = scene_page
        .find(&base.id)
        .map(scene_rect)
        .unwrap_or_else(|| "(missing)".to_string());
    let indent = "  ".repeat(depth);
    println!(
        "{indent}- id={} type={} name={:?} local=({local_x:.2},{local_y:.2},{w:.2},{h:.2}) converted_abs=({abs_x:.2},{abs_y:.2},{w:.2},{h:.2}) rendered={rendered}",
        base.id,
        variant_name(node),
        base.name.as_deref().unwrap_or("")
    );
    println!("{indent}  {}", layout_props(node));
    if matches!(node, PenNode::Text(_)) {
        if let Some(scene_node) = scene_page.find(&base.id) {
            println!(
                "{indent}  renderedText=family:{:?} size:{:.2} weight:{} lineHeight:{:.3} letterSpacing:{:.2}",
                scene_node.font_family,
                scene_node.font_size,
                scene_node.font_weight,
                scene_node.line_height,
                scene_node.letter_spacing
            );
        }
    }
    if depth < max_depth {
        for child in children_of(node) {
            dump_node(child, (abs_x, abs_y), scene_page, depth + 1, max_depth);
        }
    }
}

fn scene_rect(node: &SceneNode) -> String {
    let bounds = node.bounds;
    format!(
        "({:.2},{:.2},{:.2},{:.2})",
        bounds.origin.x, bounds.origin.y, bounds.size.x, bounds.size.y
    )
}

fn layout_props(node: &PenNode) -> String {
    let base = match node {
        PenNode::Frame(node) => format!(
            "layout={:?} gap={:?} padding={:?} justify={:?} align={:?}",
            node.container.layout,
            node.container.gap,
            node.container.padding,
            node.container.justify_content,
            node.container.align_items
        ),
        PenNode::Group(node) => format!(
            "layout={:?} gap={:?} padding={:?} justify={:?} align={:?}",
            node.container.layout,
            node.container.gap,
            node.container.padding,
            node.container.justify_content,
            node.container.align_items
        ),
        PenNode::Rectangle(node) => format!(
            "layout={:?} gap={:?} padding={:?} justify={:?} align={:?}",
            node.container.layout,
            node.container.gap,
            node.container.padding,
            node.container.justify_content,
            node.container.align_items
        ),
        PenNode::Text(node) => format!(
            "textGrowth={:?} align={:?}/{:?} family={:?} fontSize={:?} weight={:?} lineHeight={:?} letterSpacing={:?}",
            node.text_growth,
            node.text_align,
            node.text_align_vertical,
            node.font_family,
            node.font_size,
            node.font_weight,
            node.line_height,
            node.letter_spacing
        ),
        _ => "layout=-".to_string(),
    };
    match image_fill(node) {
        Some(image) => format!(
            "{base} imageMode={:?} original={:?} transform={:?}",
            image.mode, image.original_size, image.transform
        ),
        None => base,
    }
}

fn image_fill(node: &PenNode) -> Option<&jian_ops_schema::style::ImageFillBody> {
    use jian_ops_schema::style::PenFill;
    let fills = match node {
        PenNode::Frame(node) => node.container.fill.as_deref(),
        PenNode::Group(node) => node.container.fill.as_deref(),
        PenNode::Rectangle(node) => node.container.fill.as_deref(),
        PenNode::Ellipse(node) => node.fill.as_deref(),
        PenNode::Polygon(node) => node.fill.as_deref(),
        PenNode::Path(node) => node.fill.as_deref(),
        PenNode::Text(node) => node.fill.as_deref(),
        _ => None,
    }?;
    fills.iter().find_map(|fill| match fill {
        PenFill::Image(image) => Some(image),
        _ => None,
    })
}

fn numeric_size(node: &PenNode) -> (f64, f64) {
    let (width, height) = sizing(node);
    (numeric(width), numeric(height))
}

fn numeric(sizing: Option<&SizingBehavior>) -> f64 {
    match sizing {
        Some(SizingBehavior::Number(value)) => *value,
        _ => 0.0,
    }
}

fn sizing(node: &PenNode) -> (Option<&SizingBehavior>, Option<&SizingBehavior>) {
    match node {
        PenNode::Frame(node) => (
            node.container.width.as_ref(),
            node.container.height.as_ref(),
        ),
        PenNode::Group(node) => (
            node.container.width.as_ref(),
            node.container.height.as_ref(),
        ),
        PenNode::Rectangle(node) => (
            node.container.width.as_ref(),
            node.container.height.as_ref(),
        ),
        PenNode::Ellipse(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Polygon(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Path(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Text(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::TextInput(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::TextArea(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Select(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Switch(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Checkbox(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Slider(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::RadioGroup(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::NumberInput(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Progress(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Tabs(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Image(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::IconFont(node) => (node.width.as_ref(), node.height.as_ref()),
        PenNode::Line(_) | PenNode::Ref(_) => (None, None),
    }
}

fn children_of(node: &PenNode) -> &[PenNode] {
    match node {
        PenNode::Frame(node) => node.children.as_deref().unwrap_or(&[]),
        PenNode::Group(node) => node.children.as_deref().unwrap_or(&[]),
        PenNode::Rectangle(node) => node.children.as_deref().unwrap_or(&[]),
        PenNode::Tabs(node) => node.children.as_deref().unwrap_or(&[]),
        PenNode::Ref(node) => node.children.as_deref().unwrap_or(&[]),
        _ => &[],
    }
}

fn base_of(node: &PenNode) -> &PenNodeBase {
    match node {
        PenNode::Frame(node) => &node.base,
        PenNode::Group(node) => &node.base,
        PenNode::Rectangle(node) => &node.base,
        PenNode::Ellipse(node) => &node.base,
        PenNode::Line(node) => &node.base,
        PenNode::Polygon(node) => &node.base,
        PenNode::Path(node) => &node.base,
        PenNode::Text(node) => &node.base,
        PenNode::TextInput(node) => &node.base,
        PenNode::TextArea(node) => &node.base,
        PenNode::Select(node) => &node.base,
        PenNode::Switch(node) => &node.base,
        PenNode::Checkbox(node) => &node.base,
        PenNode::Slider(node) => &node.base,
        PenNode::RadioGroup(node) => &node.base,
        PenNode::NumberInput(node) => &node.base,
        PenNode::Progress(node) => &node.base,
        PenNode::Tabs(node) => &node.base,
        PenNode::Image(node) => &node.base,
        PenNode::IconFont(node) => &node.base,
        PenNode::Ref(node) => &node.base,
    }
}

fn variant_name(node: &PenNode) -> &'static str {
    match node {
        PenNode::Frame(_) => "frame",
        PenNode::Group(_) => "group",
        PenNode::Rectangle(_) => "rectangle",
        PenNode::Ellipse(_) => "ellipse",
        PenNode::Line(_) => "line",
        PenNode::Polygon(_) => "polygon",
        PenNode::Path(_) => "path",
        PenNode::Text(_) => "text",
        PenNode::TextInput(_) => "text_input",
        PenNode::TextArea(_) => "text_area",
        PenNode::Select(_) => "select",
        PenNode::Switch(_) => "switch",
        PenNode::Checkbox(_) => "checkbox",
        PenNode::Slider(_) => "slider",
        PenNode::RadioGroup(_) => "radio_group",
        PenNode::NumberInput(_) => "number_input",
        PenNode::Progress(_) => "progress",
        PenNode::Tabs(_) => "tabs",
        PenNode::Image(_) => "image",
        PenNode::IconFont(_) => "icon_font",
        PenNode::Ref(_) => "ref",
    }
}
