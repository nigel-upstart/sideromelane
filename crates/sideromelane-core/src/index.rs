use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{MarkdownNote, NoteAnalysis, NoteId};

/// In-memory derived index for a set of parsed folder notes.
///
/// ## Link resolution policy (ADR 0006)
///
/// Wiki links resolve by case-sensitive comparison against note file stems (`Path::file_stem`).
/// When multiple notes share the same stem the target is considered **ambiguous**: the edge is
/// omitted from `graph` and `backlinks`, but the conflict set is surfaced via
/// `ambiguous_targets()` so the UI can warn the user. Missing-link targets are simply not
/// added to edges or backlinks; they can be retrieved for "create new note" workflows via
/// the wiki links on each `NoteAnalysis`. Alias (`|`) and anchor (`#`) are preserved on
/// `WikiLink` but do not influence resolution in v1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FolderIndex {
    graph: Graph,
    backlinks: BTreeMap<NoteId, Vec<Backlink>>,
    /// Stems that map to more than one note. These are excluded from graph/backlink resolution.
    ambiguous_targets: BTreeMap<String, Vec<NoteId>>,
}

impl FolderIndex {
    /// Builds an index from parsed notes.
    ///
    /// Wiki links resolve by unique note file stem. Links to missing or ambiguous targets are
    /// preserved in note analysis but omitted from graph edges and backlinks.
    /// Ambiguous stems are available via [`FolderIndex::ambiguous_targets`].
    #[must_use]
    pub fn from_notes(note_sources: impl IntoIterator<Item = MarkdownNote>) -> Self {
        let mut analyzed_notes = note_sources
            .into_iter()
            .map(|note| {
                let note_id = note.note_id().clone();
                let analysis = NoteAnalysis::from_note(&note);
                (note_id, analysis)
            })
            .collect::<Vec<_>>();

        analyzed_notes.sort_by(|left, right| left.0.cmp(&right.0));

        let (stem_to_note_id, ambiguous_targets) = unique_stem_lookup(&analyzed_notes);
        let nodes = analyzed_notes
            .iter()
            .map(|(note_id, _analysis)| GraphNode {
                note_id: note_id.clone(),
            })
            .collect::<Vec<_>>();
        let mut edges = Vec::new();
        let mut backlinks: BTreeMap<NoteId, Vec<Backlink>> = BTreeMap::new();

        for (source, analysis) in &analyzed_notes {
            for link in analysis.wiki_links() {
                let Some(target) = stem_to_note_id.get(link.target()) else {
                    continue;
                };

                let edge = GraphEdge {
                    source: source.clone(),
                    target: target.clone(),
                };
                let backlink = Backlink {
                    source: source.clone(),
                    target: target.clone(),
                };

                backlinks.entry(target.clone()).or_default().push(backlink);
                edges.push(edge);
            }
        }

        Self {
            graph: Graph { nodes, edges },
            backlinks,
            ambiguous_targets,
        }
    }

    /// Returns graph data for indexed notes.
    #[must_use]
    pub const fn graph(&self) -> &Graph {
        &self.graph
    }

    /// Returns backlinks that target the provided note.
    #[must_use]
    pub fn backlinks_to(&self, note_id: &NoteId) -> &[Backlink] {
        self.backlinks.get(note_id).map_or(&[], Vec::as_slice)
    }

    /// Returns stems that matched more than one note and are therefore excluded from
    /// link resolution. The UI should surface these to warn the user of ambiguous links.
    #[must_use]
    pub const fn ambiguous_targets(&self) -> &BTreeMap<String, Vec<NoteId>> {
        &self.ambiguous_targets
    }

    /// Returns the depth-bounded neighborhood of `focus`.
    ///
    /// BFS from `focus` over both forward links (edges where source == current)
    /// and backlinks (edges where target == current), expanding up to `depth`
    /// hops. The returned [`Neighborhood`] contains every node reachable within
    /// `depth` hops together with all directed edges between any two nodes in
    /// the result set — including self-loops on `focus`.
    ///
    /// `depth == 0` yields just the focus node and any self-loops on it. An
    /// unknown `focus` (not present in this index) still returns a singleton
    /// neighborhood containing only `focus` so callers can render an isolated
    /// node without special-casing missing IDs.
    #[must_use]
    pub fn neighborhood(&self, focus: &NoteId, depth: usize) -> Neighborhood {
        // BFS frontier expansion. We index edges by source/target on demand;
        // graph sizes here are small and rebuilds happen on user navigation,
        // not per frame, so the linear scan is fine.
        let mut visited: BTreeSet<NoteId> = BTreeSet::new();
        visited.insert(focus.clone());

        let mut queue: VecDeque<(NoteId, usize)> = VecDeque::new();
        queue.push_back((focus.clone(), 0));

        while let Some((current, current_depth)) = queue.pop_front() {
            if current_depth == depth {
                continue;
            }
            for edge in &self.graph.edges {
                let neighbor = if edge.source == current {
                    Some(&edge.target)
                } else if edge.target == current {
                    Some(&edge.source)
                } else {
                    None
                };
                if let Some(neighbor) = neighbor
                    && !visited.contains(neighbor)
                {
                    visited.insert(neighbor.clone());
                    queue.push_back((neighbor.clone(), current_depth + 1));
                }
            }
        }

        let edges: Vec<(NoteId, NoteId)> = self
            .graph
            .edges
            .iter()
            .filter(|edge| visited.contains(&edge.source) && visited.contains(&edge.target))
            .map(|edge| (edge.source.clone(), edge.target.clone()))
            .collect();

        Neighborhood {
            nodes: visited.into_iter().collect(),
            edges,
        }
    }
}

/// Depth-bounded view of [`FolderIndex`] centered on a single note.
///
/// See [`FolderIndex::neighborhood`] for traversal semantics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Neighborhood {
    /// Notes within the requested hop distance, deduplicated and sorted.
    pub nodes: Vec<NoteId>,
    /// Directed edges (`source` → `target`) between any two notes in
    /// [`Neighborhood::nodes`], including self-loops.
    pub edges: Vec<(NoteId, NoteId)>,
}

/// Directed graph of note links.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Graph {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

impl Graph {
    /// Returns graph nodes in deterministic note path order.
    #[must_use]
    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    /// Returns directed graph edges in deterministic source note order.
    #[must_use]
    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }
}

/// Graph node representing a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    note_id: NoteId,
}

impl GraphNode {
    /// Returns the note represented by this graph node.
    #[must_use]
    pub const fn note_id(&self) -> &NoteId {
        &self.note_id
    }
}

/// Directed graph edge from a source note to a target note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    source: NoteId,
    target: NoteId,
}

impl GraphEdge {
    /// Returns the source note.
    #[must_use]
    pub const fn source(&self) -> &NoteId {
        &self.source
    }

    /// Returns the target note.
    #[must_use]
    pub const fn target(&self) -> &NoteId {
        &self.target
    }
}

/// Reverse link from a source note to the current target note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlink {
    source: NoteId,
    target: NoteId,
}

impl Backlink {
    /// Returns the note containing the original wiki link.
    #[must_use]
    pub const fn source(&self) -> &NoteId {
        &self.source
    }

    /// Returns the note targeted by the original wiki link.
    #[must_use]
    pub const fn target(&self) -> &NoteId {
        &self.target
    }
}

/// Returns a map of unambiguous stem→NoteId and a separate map of ambiguous stems→Vec<NoteId>.
fn unique_stem_lookup(
    analyzed_notes: &[(NoteId, NoteAnalysis)],
) -> (BTreeMap<String, NoteId>, BTreeMap<String, Vec<NoteId>>) {
    let mut stem_counts = BTreeMap::<String, usize>::new();

    for (note_id, _analysis) in analyzed_notes {
        *stem_counts
            .entry(note_id.file_stem().to_owned())
            .or_default() += 1;
    }

    let mut unique = BTreeMap::new();
    let mut ambiguous: BTreeMap<String, Vec<NoteId>> = BTreeMap::new();

    for (note_id, _analysis) in analyzed_notes {
        let stem = note_id.file_stem();
        let count = stem_counts.get(stem).copied().unwrap_or(0);
        if count == 1 {
            unique.insert(stem.to_owned(), note_id.clone());
        } else {
            ambiguous
                .entry(stem.to_owned())
                .or_default()
                .push(note_id.clone());
        }
    }

    (unique, ambiguous)
}
