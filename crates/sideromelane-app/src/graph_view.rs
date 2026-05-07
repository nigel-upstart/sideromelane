//! Graph-view paint code with pan/zoom and selection.
//!
//! Holds [`GraphViewState`] across frames so positions don't shift unless the
//! underlying graph snapshot changes.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use sideromelane_core::{FolderIndex, NoteId};

use crate::graph_layout::{LayoutParams, fruchterman_reingold};

/// Persistent graph-view UI state. Owned by the app.
#[derive(Debug, Clone)]
pub struct GraphViewState {
    /// Pan offset in screen pixels.
    pan: Vec2,
    /// Zoom factor (1.0 = base scale where the unit square fills the view).
    zoom: f32,
    /// Cached layout in unit-square coordinates, regenerated when the graph
    /// signature changes.
    positions: BTreeMap<NoteId, Pos2>,
    /// Edges captured alongside the positions.
    edges: Vec<(NoteId, NoteId)>,
    /// Hash of the graph snapshot used to produce the current positions, so
    /// we know when to recompute.
    signature: GraphSignature,
}

impl Default for GraphViewState {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            positions: BTreeMap::new(),
            edges: Vec::new(),
            signature: GraphSignature::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GraphSignature {
    nodes: Vec<NoteId>,
    edges: Vec<(NoteId, NoteId)>,
}

impl GraphSignature {
    fn from_index(folder_index: &FolderIndex) -> Self {
        let mut nodes: Vec<NoteId> = folder_index
            .graph()
            .nodes()
            .iter()
            .map(|node| node.note_id().clone())
            .collect();
        nodes.sort();
        let mut edges: Vec<(NoteId, NoteId)> = folder_index
            .graph()
            .edges()
            .iter()
            .map(|edge| (edge.source().clone(), edge.target().clone()))
            .collect();
        edges.sort();
        Self { nodes, edges }
    }
}

/// Paints the graph view into `ui` and returns the note id that was clicked,
/// if any.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut GraphViewState,
    folder_index: &FolderIndex,
    selected: Option<&NoteId>,
) -> Option<NoteId> {
    let signature = GraphSignature::from_index(folder_index);

    // Recompute layout only when the graph snapshot changes.
    if signature != state.signature {
        state.positions =
            fruchterman_reingold(&signature.nodes, &signature.edges, &LayoutParams::default());
        state.edges.clone_from(&signature.edges);
        state.signature = signature;
    }

    let desired_size = Vec2::new(ui.available_width(), 320.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    // Drag updates pan; scroll updates zoom centered on the pointer.
    if response.dragged() {
        state.pan += response.drag_delta();
    }
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll.abs() > f32::EPSILON {
            let factor = (scroll * 0.01).exp();
            state.zoom = (state.zoom * factor).clamp(0.25, 8.0);
        }
    }

    // Background.
    painter.rect_filled(rect, 4.0, Color32::from_gray(20));

    if state.positions.is_empty() {
        return None;
    }

    let to_screen = |unit: Pos2| -> Pos2 {
        // Center the unit square in `rect`, scaled by zoom and shifted by pan.
        let base = rect.width().min(rect.height()) * 0.9 * state.zoom;
        let center = rect.center();
        Pos2::new(
            (unit.x - 0.5).mul_add(base, center.x + state.pan.x),
            (unit.y - 0.5).mul_add(base, center.y + state.pan.y),
        )
    };

    // Edges first so nodes paint on top.
    for (source, target) in &state.edges {
        let Some(source_position) = state.positions.get(source).copied() else {
            continue;
        };
        let Some(target_position) = state.positions.get(target).copied() else {
            continue;
        };
        painter.line_segment(
            [to_screen(source_position), to_screen(target_position)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(140, 140, 160, 90)),
        );
    }

    // Compute degree to size nodes.
    let mut degree: BTreeMap<&NoteId, usize> = BTreeMap::new();
    for (source, target) in &state.edges {
        *degree.entry(source).or_default() += 1;
        *degree.entry(target).or_default() += 1;
    }

    // Show labels only when zoomed in or hovering.
    let labels_visible = state.zoom >= 1.5;
    let hover_position = response.hover_pos();

    let mut clicked_note: Option<NoteId> = None;

    for (note_id, unit_position) in &state.positions {
        let position = to_screen(*unit_position);
        let node_degree = degree.get(note_id).copied().unwrap_or(0);
        #[allow(clippy::cast_precision_loss)]
        let radius =
            (node_degree as f32).sqrt().mul_add(1.2, 3.0).min(14.0) * state.zoom.clamp(0.5, 1.5);
        let is_selected = selected == Some(note_id);
        let fill = if is_selected {
            Color32::from_rgb(220, 110, 110)
        } else {
            Color32::from_rgb(100, 140, 200)
        };
        painter.circle_filled(position, radius, fill);
        if is_selected {
            painter.circle_stroke(position, radius + 2.0, Stroke::new(1.5, Color32::WHITE));
        }

        let near_pointer =
            hover_position.is_some_and(|hover| hover.distance(position) <= radius + 4.0);
        if labels_visible || near_pointer {
            painter.text(
                position + Vec2::new(radius + 4.0, -radius),
                egui::Align2::LEFT_TOP,
                note_id.file_stem(),
                egui::FontId::proportional(11.0),
                Color32::from_gray(220),
            );
        }

        if response.clicked()
            && let Some(pointer_pos) = response.interact_pointer_pos()
            && pointer_pos.distance(position) <= radius + 4.0
        {
            clicked_note = Some(note_id.clone());
        }
    }

    // Reset zoom and pan with double-click.
    if response.double_clicked() {
        state.pan = Vec2::ZERO;
        state.zoom = 1.0;
    }

    clicked_note
}
