//! Graph-view widget rendered via [`egui_graphs`].
//!
//! Shows the depth-bounded neighborhood of the currently focused note or tag.
//! The underlying [`egui_graphs::Graph`] is rebuilt only when the focus or the
//! observed neighborhood signature changes; otherwise the widget reuses the
//! cached graph so node positions stay stable across frames.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Pos2};
use egui_graphs::{
    DefaultEdgeShape, DefaultNodeShape, FruchtermanReingold, FruchtermanReingoldState, Graph,
    GraphView, LayoutForceDirected, SettingsInteraction, SettingsStyle,
};
use petgraph::{Directed, stable_graph::DefaultIx};
use sideromelane_core::{FolderIndex, GraphNode, Neighborhood, NoteId};

/// Default hop radius for the focus-scoped graph view.
pub const DEFAULT_DEPTH: usize = 1;

/// Fill color used for tag nodes in the graph view (soft purple).
const TAG_NODE_COLOR: Color32 = Color32::from_rgb(160, 100, 210);

/// Concrete `egui_graphs` graph type with `GraphNode` payloads, force-directed
/// layout state, and the default node/edge shapes.
type NoteGraph = Graph<GraphNode, (), Directed, DefaultIx, DefaultNodeShape, DefaultEdgeShape>;

/// Concrete `egui_graphs` widget that pairs with [`NoteGraph`].
type NoteGraphView<'a> = GraphView<
    'a,
    GraphNode,
    (),
    Directed,
    DefaultIx,
    DefaultNodeShape,
    DefaultEdgeShape,
    FruchtermanReingoldState,
    LayoutForceDirected<FruchtermanReingold>,
>;

/// Persistent graph-view state. Owned by the app.
#[derive(Debug, Default)]
pub struct GraphViewState {
    /// Cached force-directed graph snapshot. Rebuilt when [`Self::signature`]
    /// changes; reused frame-to-frame so the layout simulation can settle.
    cached: Option<NoteGraph>,
    /// Identity of the cached snapshot — focus + neighborhood node/edge set.
    /// Comparing against this lets us skip the rebuild on most frames.
    signature: Option<NeighborhoodSignature>,
    /// Last set of selected node indices observed from the widget. Used to
    /// detect a fresh selection (i.e. a click on a node) and surface the
    /// corresponding `GraphNode` to the caller.
    last_selection: Vec<petgraph::stable_graph::NodeIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NeighborhoodSignature {
    focus: GraphNode,
    nodes: Vec<GraphNode>,
    edges: Vec<(GraphNode, GraphNode)>,
}

impl NeighborhoodSignature {
    fn new(focus: &GraphNode, neighborhood: &Neighborhood) -> Self {
        let mut nodes = neighborhood.nodes.clone();
        nodes.sort();
        let mut edges = neighborhood.edges.clone();
        edges.sort();
        Self {
            focus: focus.clone(),
            nodes,
            edges,
        }
    }
}

/// Paints the focus-scoped graph view into `ui` and returns the node that was
/// clicked, if any (note or tag).
///
/// `depth` is the hop radius used to compute the neighborhood. When `focus` is
/// `None` the view renders an empty placeholder.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut GraphViewState,
    folder_index: &FolderIndex,
    focus: Option<&GraphNode>,
    depth: usize,
) -> Option<GraphNode> {
    let Some(focus) = focus else {
        ui.centered_and_justified(|ui| {
            ui.label("Select a note to view its graph");
        });
        state.cached = None;
        state.signature = None;
        state.last_selection.clear();
        return None;
    };

    let neighborhood = folder_index.neighborhood(focus, depth);
    let signature = NeighborhoodSignature::new(focus, &neighborhood);

    if state.signature.as_ref() != Some(&signature) {
        state.cached = Some(build_graph(focus, &neighborhood));
        state.signature = Some(signature);
        state.last_selection.clear();
    }

    let graph = state.cached.as_mut()?;

    let mut widget = NoteGraphView::new(graph)
        .with_styles(&SettingsStyle::default().with_labels_always(true))
        .with_interactions(
            &SettingsInteraction::default()
                .with_node_selection_enabled(true)
                .with_dragging_enabled(true),
        );
    ui.add(&mut widget);

    detect_clicked_node(graph, &mut state.last_selection)
}

/// Builds an [`egui_graphs::Graph`] from the supplied neighborhood.
///
/// Note nodes use the default (blue) fill; tag nodes use a soft purple and the
/// label is prefixed with `#`. The focus node is seeded at the origin.
fn build_graph(focus: &GraphNode, neighborhood: &Neighborhood) -> NoteGraph {
    let mut graph = NoteGraph::new(petgraph::stable_graph::StableGraph::default());
    let mut indices = BTreeMap::new();

    for node in &neighborhood.nodes {
        let location = if node == focus {
            Pos2::ZERO
        } else {
            Pos2::new(0.0, 0.0)
        };

        let label = node_label(node);
        let is_tag = matches!(node, GraphNode::Tag { .. });
        let node_clone = node.clone();

        let idx = graph.add_node_custom(node_clone, |n| {
            n.set_label(label);
            n.set_location(location);
            if is_tag {
                n.set_color(TAG_NODE_COLOR);
            }
        });
        indices.insert(node.clone(), idx);
    }

    for (source, target) in &neighborhood.edges {
        if let (Some(source_idx), Some(target_idx)) = (indices.get(source), indices.get(target)) {
            graph.add_edge(*source_idx, *target_idx, ());
        }
    }

    graph
}

fn node_label(node: &GraphNode) -> String {
    match node {
        GraphNode::Note { note_id } => note_id.file_stem().to_owned(),
        GraphNode::Tag { tag } => format!("#{}", tag.name()),
    }
}

/// Returns the `GraphNode` of a node that became newly selected this frame, if any.
fn detect_clicked_node(
    graph: &NoteGraph,
    last_selection: &mut Vec<petgraph::stable_graph::NodeIndex>,
) -> Option<GraphNode> {
    let current = graph.selected_nodes();
    let newly_selected = current
        .iter()
        .find(|idx| !last_selection.contains(idx))
        .copied();

    last_selection.clear();
    last_selection.extend_from_slice(current);

    newly_selected.and_then(|idx| graph.node(idx).map(|node| node.payload().clone()))
}

/// Returns a `GraphNode::Note` wrapping `note_id`.
///
/// Convenience for callers that track focus as a `NoteId`.
pub fn note_focus(note_id: &NoteId) -> GraphNode {
    GraphNode::Note {
        note_id: note_id.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use sideromelane_core::Tag;

    #[test]
    fn node_label_prefixes_tag_with_hash() {
        let tag = Tag::new("kubernetes").unwrap();
        let label = node_label(&GraphNode::Tag { tag });
        assert_eq!(label, "#kubernetes");
    }

    #[test]
    fn detect_clicked_node_returns_tag_node() {
        let tag = Tag::new("kubernetes").unwrap();
        let mut graph = NoteGraph::new(petgraph::stable_graph::StableGraph::default());
        let idx = graph.add_node(GraphNode::Tag { tag: tag.clone() });
        graph.set_selected_nodes(vec![idx]);

        let result = detect_clicked_node(&graph, &mut vec![]);
        assert_eq!(result, Some(GraphNode::Tag { tag }));
    }

    #[test]
    fn detect_clicked_node_returns_none_when_nothing_selected() {
        let tag = Tag::new("foo").unwrap();
        let mut graph = NoteGraph::new(petgraph::stable_graph::StableGraph::default());
        graph.add_node(GraphNode::Tag { tag });

        let result = detect_clicked_node(&graph, &mut vec![]);
        assert!(result.is_none());
    }
}
