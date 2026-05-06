# ADR 0004: Local Hybrid Search

## Status

Accepted

## Context

The spec requires keyword search plus local embedding-based semantic search with no required network
calls. Pulling a machine-learning model into v1 adds runtime, model-storage, and supply-chain
complexity before the app shell is proven.

## Decision

Implement hybrid search in two layers:

1. Lexical search: deterministic keyword scoring over file name, title, tags, frontmatter, links,
   and body.
2. Local semantic search: deterministic hashed token-vector embeddings generated from note text and
   query text.

The semantic layer is a local embedding interface and baseline implementation. It performs no
network calls, requires no model download, and can later be replaced by a local ML embedding runtime
behind the same boundary.

## Consequences

- v1 has hybrid behavior without cloud or model distribution risk.
- Search quality is predictable but not as strong as a trained embedding model.
- The app can merge lexical and semantic scores immediately.
- A future model-backed runtime requires a separate ADR covering model source, storage, update
  policy, licensing, and performance.
