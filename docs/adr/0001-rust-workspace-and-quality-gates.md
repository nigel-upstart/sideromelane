# ADR 0001: Rust Workspace And Quality Gates

## Status

Accepted

## Context

Sideromelane is starting as a local-only native macOS desktop app using the Rust toolchain. The
product spec and GUI framework are not selected yet, but the repo needs a quality baseline before
implementation begins.

## Decision

Use a Cargo workspace with a pure `sideromelane-core` crate first. Defer the app shell and GUI
framework until the spec makes the UI and platform requirements clear.

Quality gates are:

- `cargo fmt`
- `cargo clippy` with warnings denied in CI
- `cargo test`
- `cargo doc`
- `cargo deny`
- `cargo machete`
- `typos`
- `taplo`

## Consequences

- The repo has real checks before product code exists.
- Early domain logic can be tested without macOS UI dependencies.
- GUI selection remains an explicit architecture decision instead of an accidental first dependency.

