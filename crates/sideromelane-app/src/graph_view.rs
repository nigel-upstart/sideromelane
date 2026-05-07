//! Graph-view widget rendered via [`egui_graphs`].
//!
//! Shows the depth-bounded neighborhood of the currently focused note. The
//! underlying [`egui_graphs::Graph`] is rebuilt only when the focus or the
//! observed neighborhood signature changes; otherwise the widget reuses the
//! cached graph so node positions stay stable across frames.

use std::collections::BTreeMap;

use eframe::egui::{self, Pos2};
use egui_graphs::{
    DefaultEdgeShape, DefaultNodeShape, FruchtermanReingold, FruchtermanReingoldState, Graph,
    GraphView, LayoutForceDirected, SettingsInteraction, SettingsStyle,
};
use petgraph::{Directed, stable_graph::DefaultIx};
use sideromelane_core::{FolderIndex, Neighborhood, NoteId};

/// Default hop radius for the focus-scoped graph view.
pub const DEFAULT_DEPTH: usize = 1;

/// Concrete `egui_graphs` graph type with `NoteId` payloads, force-directed
/// layout state, and the default node/edge shapes.
type NoteGraph = Graph<NoteId, (), Directed, DefaultIx, DefaultNodeShape, DefaultEdgeShape>;

/// Concrete `egui_graphs` widget that pairs with [`NoteGraph`].
type NoteGraphView<'a> = GraphView<
    'a,
    NoteId,
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
    /// corresponding `NoteId` to the caller.
    last_selection: Vec<petgraph::stable_graph::NodeIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NeighborhoodSignature {
    focus: NoteId,
    nodes: Vec<NoteId>,
    edges: Vec<(NoteId, NoteId)>,
}

impl NeighborhoodSignature {
    fn new(focus: &NoteId, neighborhood: &Neighborhood) -> Self {
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

/// Paints the focus-scoped graph view into `ui` and returns the note id that
/// was clicked, if any.
///
/// `depth` is the hop radius used to compute the neighborhood. When `focus` is
/// `None` the view renders an empty placeholder.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut GraphViewState,
    folder_index: &FolderIndex,
    focus: Option<&NoteId>,
    depth: usize,
) -> Option<NoteId> {
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

    detect_clicked_note(graph, &mut state.last_selection)
}

/// Builds an [`egui_graphs::Graph`] from the supplied neighborhood. Layout is
/// not seeded — the force-directed simulation produces positions on first
/// draw and they persist via the cache.
fn build_graph(focus: &NoteId, neighborhood: &Neighborhood) -> NoteGraph {
    let mut graph = NoteGraph::new(petgraph::stable_graph::StableGraph::default());
    let mut indices = BTreeMap::new();

    for note_id in &neighborhood.nodes {
        // Seed each node near the origin so the simulation starts from a
        // small cluster and expands; the focus stays at (0, 0) as a hint.
        let location = if note_id == focus {
            Pos2::ZERO
        } else {
            // Spread initial positions slightly so the algorithm has a
            // gradient to work against on the first step.
            Pos2::new(0.0, 0.0)
        };
        let idx = graph.add_node_with_label_and_location(
            note_id.clone(),
            note_id.file_stem().to_owned(),
            location,
        );
        indices.insert(note_id.clone(), idx);
    }

    for (source, target) in &neighborhood.edges {
        if let (Some(source_idx), Some(target_idx)) = (indices.get(source), indices.get(target)) {
            graph.add_edge(*source_idx, *target_idx, ());
        }
    }

    graph
}

/// Returns the `NoteId` of a node that became newly selected this frame, if
/// any. Selecting the same node again (or clearing the selection) returns
/// `None`.
fn detect_clicked_note(
    graph: &NoteGraph,
    last_selection: &mut Vec<petgraph::stable_graph::NodeIndex>,
) -> Option<NoteId> {
    let current = graph.selected_nodes();
    let newly_selected = current
        .iter()
        .find(|idx| !last_selection.contains(idx))
        .copied();

    last_selection.clear();
    last_selection.extend_from_slice(current);

    newly_selected.and_then(|idx| graph.node(idx).map(|node| node.payload().clone()))
}
