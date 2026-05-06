# Rust Quality Gates

## Required Local Checks

```sh
just check
```

This runs:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --workspace --all-features --no-deps`

## Supply Chain And Static Analysis

```sh
just audit
```

This runs:

- `cargo deny check`
- `cargo machete`
- `typos`
- `taplo fmt --check`

Install those tools with:

```sh
just install-tools
```

## Git Hooks

This repo uses `lefthook` for local hook orchestration.

```sh
just install-hooks
just hooks
```

- `pre-commit`: format check, clippy, and tests.
- `pre-push`: full `just check` and `just audit`.

## Dependency Rules

- Prefer the Rust standard library and existing workspace crates.
- Ask before adding a dependency.
- Prefer crates that are actively maintained, documented, and narrowly scoped.
- Check licenses and advisories before accepting a dependency.
- Do not use wildcard dependency versions.

## macOS App Rules

- Keep pure domain logic in crates that do not depend on macOS APIs.
- Put platform adapters behind explicit module boundaries.
- Any sandboxing, file access, signing, or notarization decision belongs in an ADR.
- Do not add network capability unless the spec explicitly requires it.
