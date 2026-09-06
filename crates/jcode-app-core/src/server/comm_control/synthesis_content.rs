//! Synthesis content.

/// Assignment content for a (re-)dispatched node.
///
/// For a re-woken composite (`is_composite_synthesis`), the node's original
/// content is the now-stale decomposition brief, so replace it with an explicit
/// synthesis instruction that tells the planner to integrate its children and
/// finish with `complete_node`. Otherwise the original content is used verbatim.
pub(super) fn composite_synthesis_content(
    item_id: &str,
    raw_content: &str,
    is_composite_synthesis: bool,
) -> String {
    if is_composite_synthesis {
        format!(
            "Synthesis turn for composite node '{item_id}'. Its children (and the deep-mode \
             critique/verify gate) are complete; their outputs are provided below. Read them, \
             write one synthesized result, and finish by calling `swarm complete_node` with \
             node_id=\"{item_id}\" and an artifact summarizing the integrated findings. Do NOT \
             call expand_node again. Original brief: {raw_content}"
        )
    } else {
        raw_content.to_string()
    }
}
