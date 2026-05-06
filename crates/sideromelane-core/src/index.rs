use std::collections::BTreeMap;

use crate::{MarkdownNote, NoteAnalysis, NoteId};

/// In-memory derived index for a set of parsed folder notes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FolderIndex {
    graph: Graph,
    backlinks: BTreeMap<NoteId, Vec<Backlink>>,
}

impl FolderIndex {
    /// Builds an index from parsed notes.
    ///
    /// Wiki links resolve by unique note file stem. Links to missing or ambiguous targets are
    /// preserved in note analysis but omitted from graph edges and backlinks.
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

        let stem_to_note_id = unique_stem_lookup(&analyzed_notes);
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

fn unique_stem_lookup(analyzed_notes: &[(NoteId, NoteAnalysis)]) -> BTreeMap<String, NoteId> {
    let mut stem_counts = BTreeMap::<String, usize>::new();

    for (note_id, _analysis) in analyzed_notes {
        *stem_counts
            .entry(note_id.file_stem().to_owned())
            .or_default() += 1;
    }

    analyzed_notes
        .iter()
        .filter_map(|(note_id, _analysis)| {
            let stem = note_id.file_stem();

            if stem_counts.get(stem) == Some(&1) {
                Some((stem.to_owned(), note_id.clone()))
            } else {
                None
            }
        })
        .collect()
}
