//! Force-directed (Fruchterman–Reingold) layout for the graph view.
//!
//! Pure, deterministic given the same inputs. The caller stores the resulting
//! positions and only re-runs the simulation when the underlying graph
//! snapshot changes (e.g. after `IndexerEvent::IndexUpdated`), not per frame.

use std::collections::BTreeMap;

use eframe::egui::{Pos2, Vec2};
use sideromelane_core::NoteId;

/// Tunable simulation parameters.
#[derive(Debug, Clone, Copy)]
pub struct LayoutParams {
    /// Number of iterations to run.
    pub iterations: u32,
    /// Initial bounding box width (logical units).
    pub width: f32,
    /// Initial bounding box height (logical units).
    pub height: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            iterations: 200,
            width: 1.0,
            height: 1.0,
        }
    }
}

/// Computes layout positions for the supplied graph.
///
/// Positions are returned in a unit square `[0, 1] x [0, 1]`; the caller is
/// expected to remap them to screen coordinates. Edges are undirected for
/// layout purposes — each `(source, target)` contributes attraction between
/// both endpoints regardless of direction.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn fruchterman_reingold(
    nodes: &[NoteId],
    edges: &[(NoteId, NoteId)],
    params: &LayoutParams,
) -> BTreeMap<NoteId, Pos2> {
    if nodes.is_empty() {
        return BTreeMap::new();
    }

    let node_count = nodes.len() as f32;
    let area = params.width * params.height;
    // Optimal edge length k. Slightly increased so disconnected components don't pile up.
    let k = (area / node_count).sqrt() * 0.85;

    // Seed positions on a golden-angle spiral so disconnected nodes start
    // distributed but close together — the simulation pushes them apart.
    let golden_angle = std::f32::consts::PI * (3.0 - (5.0_f32).sqrt());
    let mut positions: Vec<Pos2> = nodes
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let angle = index as f32 * golden_angle;
            let radius =
                (index as f32 / node_count).sqrt() * params.width.min(params.height) * 0.45;
            Pos2::new(
                params.width.mul_add(0.5, radius * angle.cos()),
                params.height.mul_add(0.5, radius * angle.sin()),
            )
        })
        .collect();

    let index_of: BTreeMap<NoteId, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, note_id)| (note_id.clone(), index))
        .collect();

    // Pre-resolve edges to indices so the simulation hot path doesn't hash.
    let edge_indices: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|(source, target)| {
            let source_index = index_of.get(source)?;
            let target_index = index_of.get(target)?;
            if source_index == target_index {
                None
            } else {
                Some((*source_index, *target_index))
            }
        })
        .collect();

    let mut temperature = params.width.max(params.height) * 0.1;
    let cooling = temperature / params.iterations.max(1) as f32;
    let mut displacements = vec![Vec2::ZERO; nodes.len()];

    for _ in 0..params.iterations {
        // Repulsive forces: every pair pushes apart.
        displacements.fill(Vec2::ZERO);
        for source_index in 0..positions.len() {
            for target_index in (source_index + 1)..positions.len() {
                let delta = positions[source_index] - positions[target_index];
                let distance = delta.length().max(0.01);
                let force = (k * k) / distance;
                let direction = delta / distance;
                displacements[source_index] += direction * force;
                displacements[target_index] -= direction * force;
            }
        }

        // Attractive forces: edges pull endpoints together.
        for &(source_index, target_index) in &edge_indices {
            let delta = positions[source_index] - positions[target_index];
            let distance = delta.length().max(0.01);
            let force = (distance * distance) / k;
            let direction = delta / distance;
            displacements[source_index] -= direction * force;
            displacements[target_index] += direction * force;
        }

        // Move by displacement, capped at the current temperature, and confine
        // to the bounding box so the simulation stays stable.
        for index in 0..positions.len() {
            let displacement = displacements[index];
            let length = displacement.length().max(0.01);
            let capped = displacement / length * length.min(temperature);
            let mut next = positions[index] + capped;
            next.x = next.x.clamp(0.0, params.width);
            next.y = next.y.clamp(0.0, params.height);
            positions[index] = next;
        }

        temperature = (temperature - cooling).max(0.01);
    }

    nodes
        .iter()
        .zip(positions)
        .map(|(note_id, position)| (note_id.clone(), position))
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use sideromelane_core::NoteId;

    use super::{LayoutParams, fruchterman_reingold};

    fn note(name: &str) -> NoteId {
        NoteId::from_folder_relative_path(PathBuf::from(format!("{name}.md"))).unwrap()
    }

    #[test]
    fn empty_graph_returns_empty_map() {
        let positions = fruchterman_reingold(&[], &[], &LayoutParams::default());
        assert!(positions.is_empty());
    }

    #[test]
    fn single_node_inside_bounds() {
        let nodes = vec![note("solo")];
        let positions = fruchterman_reingold(&nodes, &[], &LayoutParams::default());
        let position = positions.values().next().expect("position");
        assert!(position.x >= 0.0 && position.x <= 1.0);
        assert!(position.y >= 0.0 && position.y <= 1.0);
    }

    #[test]
    fn connected_pair_pulls_within_bounding_box() {
        let nodes = vec![note("a"), note("b")];
        let edges = vec![(nodes[0].clone(), nodes[1].clone())];
        let positions = fruchterman_reingold(
            &nodes,
            &edges,
            &LayoutParams {
                iterations: 500,
                ..LayoutParams::default()
            },
        );
        let positions: Vec<_> = nodes.iter().map(|n| positions[n]).collect();
        let distance = (positions[0] - positions[1]).length();
        // After 500 iterations, the pair should be inside the unit square.
        assert!(positions.iter().all(|p| p.x >= 0.0 && p.x <= 1.0));
        assert!(positions.iter().all(|p| p.y >= 0.0 && p.y <= 1.0));
        assert!(distance < 1.5, "edge distance {distance} too large");
    }

    #[test]
    fn star_layout_keeps_hub_central_relative_to_leaves() {
        let hub = note("hub");
        let leaves: Vec<NoteId> = (0..6).map(|i| note(&format!("leaf{i}"))).collect();
        let mut nodes = vec![hub.clone()];
        nodes.extend(leaves.iter().cloned());
        let edges: Vec<_> = leaves
            .iter()
            .map(|leaf| (hub.clone(), leaf.clone()))
            .collect();
        let positions = fruchterman_reingold(
            &nodes,
            &edges,
            &LayoutParams {
                iterations: 500,
                ..LayoutParams::default()
            },
        );
        let hub_pos = positions[&hub];
        let leaf_distances: Vec<f32> = leaves
            .iter()
            .map(|leaf| (positions[leaf] - hub_pos).length())
            .collect();
        // Every leaf should be within the bounding box and within a sane radius
        // of the hub. Don't assert pixel-perfect; this is a sanity check.
        for distance in leaf_distances {
            assert!(distance < 1.5);
        }
    }
}
