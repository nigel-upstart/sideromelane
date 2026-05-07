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
    GraphView, LayoutForceDirected, SettingsInteraction, SettingsStyle, reset,
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

    let graph_changed = state.signature.as_ref() != Some(&signature);
    if graph_changed {
        state.cached = Some(build_graph(focus, &neighborhood));
        state.signature = Some(signature);
        state.last_selection.clear();
    }

    let graph = state.cached.as_mut()?;

    if graph_changed {
        reset::<FruchtermanReingoldState>(ui, None);
        NoteGraphView::fast_forward_budgeted(ui, graph, 300, 50, None);
    }

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
/// label is prefixed with `#`. The focus node is seeded at the origin; all
/// others are placed on an evenly-spaced circle so the force-directed layout
/// has well-defined initial forces to work with.
fn build_graph(focus: &GraphNode, neighborhood: &Neighborhood) -> NoteGraph {
    let mut graph = NoteGraph::new(petgraph::stable_graph::StableGraph::default());
    let mut indices = BTreeMap::new();

    let non_focus: Vec<&GraphNode> = neighborhood.nodes.iter().filter(|n| *n != focus).collect();
    let count = non_focus.len().max(1);

    for node in &neighborhood.nodes {
        let location = if node == focus {
            Pos2::ZERO
        } else {
            let i = non_focus.iter().position(|n| *n == node).unwrap_or(0);
            // Node counts never exceed a few thousand; precision loss is negligible.
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let angle = 2.0_f32 * std::f32::consts::PI * i as f32 / count as f32;
            Pos2::new(150.0 * angle.cos(), 150.0 * angle.sin())
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
    use sideromelane_core::{Neighborhood, Tag};

    fn note_node(path: &str) -> GraphNode {
        GraphNode::Note {
            note_id: NoteId::from_folder_relative_path(path).unwrap(),
        }
    }

    fn tag_node(name: &str) -> GraphNode {
        GraphNode::Tag {
            tag: Tag::new(name).unwrap(),
        }
    }

    fn node_location(graph: &NoteGraph, target: &GraphNode) -> Option<Pos2> {
        graph
            .nodes_iter()
            .find(|(_, n)| n.payload() == target)
            .map(|(_, n)| n.location())
    }

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

    #[test]
    fn node_label_returns_file_stem_for_note_node() {
        let note_id = NoteId::from_folder_relative_path("work/My Note.md").unwrap();
        let label = node_label(&GraphNode::Note { note_id });
        assert_eq!(label, "My Note");
    }

    #[test]
    fn build_graph_places_focus_node_at_origin() {
        let focus = note_node("notes/Focus.md");
        let neighborhood = Neighborhood {
            nodes: vec![focus.clone()],
            edges: vec![],
        };
        let graph = build_graph(&focus, &neighborhood);
        let loc = node_location(&graph, &focus).unwrap();
        assert_eq!(loc, Pos2::ZERO);
    }

    #[test]
    fn build_graph_places_single_non_focus_node_off_origin() {
        let focus = note_node("notes/Focus.md");
        let other = tag_node("kubernetes");
        let neighborhood = Neighborhood {
            nodes: vec![focus.clone(), other.clone()],
            edges: vec![(focus.clone(), other.clone())],
        };
        let graph = build_graph(&focus, &neighborhood);

        let focus_loc = node_location(&graph, &focus).unwrap();
        let other_loc = node_location(&graph, &other).unwrap();

        assert_eq!(focus_loc, Pos2::ZERO);
        assert_ne!(
            other_loc,
            Pos2::ZERO,
            "non-focus node must not be at origin"
        );
        // With count=1, angle=0 → (150, 0)
        assert!((other_loc.x - 150.0_f32).abs() < 1e-3);
        assert!(other_loc.y.abs() < 1e-3);
    }

    #[test]
    fn build_graph_places_multiple_non_focus_nodes_at_distinct_positions() {
        let focus = note_node("notes/Focus.md");
        let t1 = tag_node("alpha");
        let t2 = tag_node("beta");
        let t3 = tag_node("gamma");
        let neighborhood = Neighborhood {
            nodes: vec![focus.clone(), t1.clone(), t2.clone(), t3.clone()],
            edges: vec![],
        };
        let graph = build_graph(&focus, &neighborhood);

        let locs: Vec<Pos2> = [&t1, &t2, &t3]
            .iter()
            .map(|n| node_location(&graph, n).unwrap())
            .collect();

        // All off origin
        for loc in &locs {
            assert_ne!(*loc, Pos2::ZERO, "non-focus nodes must not be at origin");
        }

        // All on a circle of radius ~150
        for loc in &locs {
            let r = loc.x.hypot(loc.y);
            assert!(
                (r - 150.0_f32).abs() < 1e-2,
                "expected radius ~150, got {r}"
            );
        }

        // All distinct
        assert_ne!(locs[0], locs[1]);
        assert_ne!(locs[1], locs[2]);
        assert_ne!(locs[0], locs[2]);
    }

    #[test]
    fn build_graph_adds_edges_between_existing_nodes() {
        let focus = note_node("notes/Focus.md");
        let other = note_node("notes/Other.md");
        let neighborhood = Neighborhood {
            nodes: vec![focus.clone(), other.clone()],
            edges: vec![(focus.clone(), other)],
        };
        let graph = build_graph(&focus, &neighborhood);
        assert_eq!(graph.g().edge_count(), 1);
    }
}
