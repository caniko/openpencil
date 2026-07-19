use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ByteFingerprint {
    hash: u64,
    len: usize,
}

impl ByteFingerprint {
    fn of(bytes: &[u8]) -> Self {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = OFFSET;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        Self {
            hash,
            len: bytes.len(),
        }
    }
}

/// Thumbnails are created before op-figma materializes final data URLs. Keep
/// them transiently under the bytes that the resolver will encode, then bind
/// them only after the imported document contains those actual source strings.
#[derive(Default)]
pub(super) struct PendingImportThumbs {
    by_final_bytes: HashMap<ByteFingerprint, Vec<u8>>,
}

impl PendingImportThumbs {
    pub(super) fn record(&mut self, final_bytes: &[u8], thumbnail: Vec<u8>) {
        self.by_final_bytes
            .entry(ByteFingerprint::of(final_bytes))
            .or_insert(thumbnail);
    }

    fn thumbnail_for(&self, final_bytes: &[u8]) -> Option<Vec<u8>> {
        self.by_final_bytes
            .get(&ByteFingerprint::of(final_bytes))
            .cloned()
    }
}

fn decode_base64_data_url(src: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;

    let body = src.strip_prefix("data:")?;
    let (metadata, payload) = body.split_once(',')?;
    metadata
        .split(';')
        .skip(1)
        .any(|part| part.eq_ignore_ascii_case("base64"))
        .then_some(())?;
    B64.decode(payload).ok()
}

pub(super) fn bind_import_thumbnails(
    document: &jian_ops_schema::document::PenDocument,
    pending: &mut PendingImportThumbs,
) {
    use jian_ops_schema::node::image_src::paint_image_id;

    let mut seen_ids = HashSet::new();
    visit_import_image_sources(document, &mut |src| {
        let paint_id = paint_image_id(src);
        if !seen_ids.insert(paint_id) {
            return;
        }
        let Some(bytes) = decode_base64_data_url(src) else {
            return;
        };
        if let Some(thumbnail) = pending.thumbnail_for(&bytes) {
            jian_ops_schema::image_thumbs::store_thumb(paint_id, thumbnail);
        }
    });
}

/// Visit the exact typed image sources op-figma's blob resolver can create.
/// Keeping this typed avoids serializing a large imported document a second
/// time merely to discover the final source strings.
fn visit_import_image_sources(
    document: &jian_ops_schema::document::PenDocument,
    f: &mut impl FnMut(&str),
) {
    if let Some(pages) = &document.pages {
        for page in pages {
            for node in &page.children {
                visit_import_node(node, f);
            }
        }
    }
    for node in &document.children {
        visit_import_node(node, f);
    }
}

fn visit_import_fills(
    fills: &Option<Vec<jian_ops_schema::style::PenFill>>,
    f: &mut impl FnMut(&str),
) {
    let Some(fills) = fills else { return };
    for fill in fills {
        if let jian_ops_schema::style::PenFill::Image(image) = fill {
            f(image.url.as_str());
        }
    }
}

fn visit_import_stroke(
    stroke: &Option<jian_ops_schema::style::PenStroke>,
    f: &mut impl FnMut(&str),
) {
    if let Some(stroke) = stroke {
        visit_import_fills(&stroke.fill, f);
    }
}

fn visit_import_states(
    states: &Option<jian_ops_schema::state_override::WidgetStates>,
    f: &mut impl FnMut(&str),
) {
    let Some(states) = states else { return };
    for style in [
        &states.hover,
        &states.pressed,
        &states.focused,
        &states.disabled,
    ]
    .into_iter()
    .flatten()
    {
        visit_import_fills(&style.fill, f);
        visit_import_stroke(&style.stroke, f);
    }
}

fn visit_import_widget_style(
    fills: &Option<Vec<jian_ops_schema::style::PenFill>>,
    stroke: &Option<jian_ops_schema::style::PenStroke>,
    states: &Option<jian_ops_schema::state_override::WidgetStates>,
    f: &mut impl FnMut(&str),
) {
    visit_import_fills(fills, f);
    visit_import_stroke(stroke, f);
    visit_import_states(states, f);
}

fn visit_import_node(node: &jian_ops_schema::node::PenNode, f: &mut impl FnMut(&str)) {
    use jian_ops_schema::node::PenNode;

    let children = match node {
        PenNode::Frame(node) => {
            visit_import_fills(&node.container.fill, f);
            visit_import_stroke(&node.container.stroke, f);
            node.children.as_ref()
        }
        PenNode::Group(node) => {
            visit_import_fills(&node.container.fill, f);
            visit_import_stroke(&node.container.stroke, f);
            node.children.as_ref()
        }
        PenNode::Rectangle(node) => {
            visit_import_fills(&node.container.fill, f);
            visit_import_stroke(&node.container.stroke, f);
            node.children.as_ref()
        }
        PenNode::Ellipse(node) => {
            visit_import_fills(&node.fill, f);
            visit_import_stroke(&node.stroke, f);
            None
        }
        PenNode::Line(node) => {
            visit_import_stroke(&node.stroke, f);
            None
        }
        PenNode::Polygon(node) => {
            visit_import_fills(&node.fill, f);
            visit_import_stroke(&node.stroke, f);
            None
        }
        PenNode::Path(node) => {
            visit_import_fills(&node.fill, f);
            visit_import_stroke(&node.stroke, f);
            None
        }
        PenNode::Text(node) => {
            visit_import_fills(&node.fill, f);
            None
        }
        PenNode::TextInput(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Image(node) => {
            f(node.src.as_str());
            None
        }
        PenNode::IconFont(node) => {
            visit_import_fills(&node.fill, f);
            visit_import_stroke(&node.stroke, f);
            None
        }
        PenNode::TextArea(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Select(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Switch(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Checkbox(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Slider(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::RadioGroup(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::NumberInput(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Progress(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            None
        }
        PenNode::Tabs(node) => {
            visit_import_widget_style(&node.fill, &node.stroke, &node.states, f);
            node.children.as_ref()
        }
        PenNode::Ref(node) => node.children.as_ref(),
    };
    if let Some(children) = children {
        for child in children {
            visit_import_node(child, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_every_typed_image_source_shape_used_by_imports() {
        let document = serde_json::from_value(serde_json::json!({
            "version": "0.8.2",
            "children": [
                {
                    "type": "rectangle",
                    "id": "primary-fill",
                    "fill": [{"type": "image", "url": "primary"}]
                },
                {
                    "type": "line",
                    "id": "stroke-fill",
                    "stroke": {
                        "thickness": 1,
                        "fill": [{"type": "image", "url": "stroke"}]
                    }
                },
                {
                    "type": "text_input",
                    "id": "widget-state-fill",
                    "states": {
                        "hover": {
                            "fill": [{"type": "image", "url": "state"}]
                        }
                    }
                },
                {
                    "type": "frame",
                    "id": "nested-parent",
                    "children": [
                        {"type": "image", "id": "image-node", "src": "nested-image"}
                    ]
                }
            ]
        }))
        .expect("typed document");
        let mut sources = Vec::new();

        visit_import_image_sources(&document, &mut |source| sources.push(source.to_owned()));
        sources.sort();

        assert_eq!(sources, ["nested-image", "primary", "state", "stroke"]);
    }

    #[test]
    fn identical_final_bytes_bind_every_distinct_final_source_id() {
        use jian_ops_schema::node::image_src::paint_image_id;

        let png_src = "data:image/png;base64,U0FNRQ==";
        let jpeg_src = "data:image/jpeg;base64,U0FNRQ==";
        let document = serde_json::from_value(serde_json::json!({
            "version": "0.8.2",
            "children": [
                {"type": "image", "id": "png", "src": png_src},
                {"type": "image", "id": "jpeg", "src": jpeg_src}
            ]
        }))
        .expect("typed document");
        let thumbnail = vec![0xff, 0xd8, 0xff, 0xd9];
        let mut pending = PendingImportThumbs::default();
        pending.record(b"SAME", thumbnail.clone());

        bind_import_thumbnails(&document, &mut pending);

        assert_eq!(
            &*jian_ops_schema::image_thumbs::thumb_for(paint_image_id(png_src))
                .expect("PNG source thumbnail"),
            thumbnail
        );
        assert_eq!(
            &*jian_ops_schema::image_thumbs::thumb_for(paint_image_id(jpeg_src))
                .expect("JPEG source thumbnail"),
            thumbnail
        );
    }
}
