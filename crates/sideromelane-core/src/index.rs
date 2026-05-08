use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::note::Tag;
use crate::{MarkdownNote, NoteAnalysis, NoteId, merged_tags};

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
    /// Maps each tag to the notes that use it (frontmatter or inline).
    tag_index: BTreeMap<Tag, Vec<NoteId>>,
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
                (note_id, note, analysis)
            })
            .collect::<Vec<_>>();

        analyzed_notes.sort_by(|left, right| left.0.cmp(&right.0));

        let note_id_analysis: Vec<(NoteId, NoteAnalysis)> = analyzed_notes
            .iter()
            .map(|(id, _, analysis)| (id.clone(), analysis.clone()))
            .collect();

        let (stem_to_note_id, ambiguous_targets) = unique_stem_lookup(&note_id_analysis);

        // Build note nodes.
        let mut nodes: Vec<GraphNode> = analyzed_notes
            .iter()
            .map(|(note_id, _, _)| GraphNode::Note {
                note_id: note_id.clone(),
            })
            .collect();

        let mut edges = Vec::new();
        let mut backlinks: BTreeMap<NoteId, Vec<Backlink>> = BTreeMap::new();
        let mut tag_index: BTreeMap<Tag, Vec<NoteId>> = BTreeMap::new();

        for (source, note, analysis) in &analyzed_notes {
            // Wiki-link edges (note → note).
            for link in analysis.wiki_links() {
                let Some(target) = stem_to_note_id.get(link.target()) else {
                    continue;
                };

                let edge = GraphEdge {
                    source: GraphNode::Note {
                        note_id: source.clone(),
                    },
                    target: GraphNode::Note {
                        note_id: target.clone(),
                    },
                };
                let backlink = Backlink {
                    source: source.clone(),
                    target: target.clone(),
                };

                backlinks.entry(target.clone()).or_default().push(backlink);
                edges.push(edge);
            }

            // Tag edges (note → tag) and tag_index.
            for tag in merged_tags(note, analysis) {
                tag_index
                    .entry(tag.clone())
                    .or_default()
                    .push(source.clone());

                edges.push(GraphEdge {
                    source: GraphNode::Note {
                        note_id: source.clone(),
                    },
                    target: GraphNode::Tag { tag },
                });
            }
        }

        // Add tag nodes (one per unique tag).
        for tag in tag_index.keys() {
            nodes.push(GraphNode::Tag { tag: tag.clone() });
        }

        // Prune empty tag_index entries (shouldn't occur, but be defensive).
        tag_index.retain(|_, notes| !notes.is_empty());

        Self {
            graph: Graph { nodes, edges },
            backlinks,
            ambiguous_targets,
            tag_index,
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

    /// Returns a map from each tag to the notes that use it.
    ///
    /// Tags with zero notes are not present (pruned during construction).
    #[must_use]
    pub const fn tag_index(&self) -> &BTreeMap<Tag, Vec<NoteId>> {
        &self.tag_index
    }

    /// Returns the depth-bounded neighborhood of `focus`.
    ///
    /// BFS from `focus` over both forward edges and reverse edges, expanding up
    /// to `depth` hops. The returned [`Neighborhood`] contains every node
    /// reachable within `depth` hops together with all directed edges between
    /// any two nodes in the result set — including self-loops on `focus`.
    ///
    /// `depth == 0` yields just the focus node and any self-loops on it. An
    /// unknown `focus` (not present in this index) still returns a singleton
    /// neighborhood containing only `focus` so callers can render an isolated
    /// node without special-casing missing IDs.
    #[must_use]
    pub fn neighborhood(&self, focus: &GraphNode, depth: usize) -> Neighborhood {
        let mut visited: BTreeSet<GraphNode> = BTreeSet::new();
        visited.insert(focus.clone());

        let mut queue: VecDeque<(GraphNode, usize)> = VecDeque::new();
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

        let edges: Vec<(GraphNode, GraphNode)> = self
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

    /// Returns file stems of every note in the index, in deterministic order.
    pub fn note_stems(&self) -> impl Iterator<Item = &str> {
        self.graph.nodes().iter().filter_map(|n| match n {
            GraphNode::Note { note_id } => Some(note_id.file_stem()),
            GraphNode::Tag { .. } => None,
        })
    }
}

/// Depth-bounded view of [`FolderIndex`] centered on a single node.
///
/// See [`FolderIndex::neighborhood`] for traversal semantics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Neighborhood {
    /// Notes and tags within the requested hop distance, deduplicated and sorted.
    pub nodes: Vec<GraphNode>,
    /// Directed edges (`source` → `target`) between any two nodes in
    /// [`Neighborhood::nodes`], including self-loops.
    pub edges: Vec<(GraphNode, GraphNode)>,
}

/// Directed graph of note links and tag associations.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Graph {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
}

impl Graph {
    /// Returns graph nodes in deterministic order (notes first, then tags).
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

/// A node in the folder graph — either a note or a tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GraphNode {
    /// A note node, identified by its [`NoteId`].
    Note {
        /// The note identifier.
        note_id: NoteId,
    },
    /// A tag node, identified by its [`Tag`].
    Tag {
        /// The tag value.
        tag: Tag,
    },
}

impl GraphNode {
    /// Returns the [`NoteId`] if this is a note node.
    #[must_use]
    pub const fn as_note(&self) -> Option<&NoteId> {
        match self {
            Self::Note { note_id } => Some(note_id),
            Self::Tag { .. } => None,
        }
    }

    /// Returns the [`Tag`] if this is a tag node.
    #[must_use]
    pub const fn as_tag(&self) -> Option<&Tag> {
        match self {
            Self::Note { .. } => None,
            Self::Tag { tag } => Some(tag),
        }
    }
}

/// Directed graph edge from a source node to a target node.
///
/// In practice, the source is always a note and the target is either a note
/// (wiki-link edge) or a tag (tag-association edge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    source: GraphNode,
    target: GraphNode,
}

impl GraphEdge {
    /// Returns the source node.
    #[must_use]
    pub const fn source(&self) -> &GraphNode {
        &self.source
    }

    /// Returns the target node.
    #[must_use]
    pub const fn target(&self) -> &GraphNode {
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
