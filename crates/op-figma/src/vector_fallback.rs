//! Narrow vector fallbacks for structures whose rendered geometry is
//! stored on a child rather than on the converted node itself.

use crate::figma_types::BlobOrString;
use crate::mappers::{map_figma_fills, map_figma_stroke};
use crate::tree::TreeNode;
use crate::vector_decoder::{decode_vector_network_blob, DecodedVectorPath};
use jian_ops_schema::style::PenStroke;

/// A one-child boolean has the same geometry as its child. Some Figma
/// component icons store an empty geometry stub on the boolean and the
/// actual outline in the child's vector network. Recover that outline
/// while retaining the boolean's result paint as the stroke colour.
pub fn decode_single_child_boolean(
    tree: &TreeNode,
    blobs: &[BlobOrString],
) -> Option<(DecodedVectorPath, PenStroke)> {
    if tree.figma.get_str("type") != Some("BOOLEAN_OPERATION") || tree.children.len() != 1 {
        return None;
    }
    let child = &tree.children[0];
    if child.figma.get_str("type") != Some("VECTOR") {
        return None;
    }

    let decoded = decode_vector_network_blob(&child.figma, blobs)?;
    let mut stroke = map_figma_stroke(&child.figma)?;
    if let Some(parent_fill) = map_figma_fills(tree.figma.get_array("fillPaints")) {
        stroke.fill = Some(parent_fill);
    }
    Some((decoded, stroke))
}
