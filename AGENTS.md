# Codex Common Rules

In `~/repos`, repositories generally follow `{owner}/{repo}`. Personal repositories live under
`~/repos/nigel-upstart`; organization repositories commonly live under `~/repos/teamupstart`.

## Python

- Prefer `uv` over `pip`.
- Prefer `uv run` over invoking `python` directly when running project code.
- If package resolution is flaky, try `--index-strategy unsafe-best-match`.

## Skills

This repo symlinks `~/.codex/skills` to the repo `skills/` directory.
`npx skills add` installs content into `~/.agents/skills`, so run `just link-skills` after adding a
new external skill to import it into this repo and expose it through `~/.codex/skills`.

The default imported skill set for this project is intentionally small:

- `using-agent-skills`
- `spec-driven-development`
- `planning-and-task-breakdown`
- `incremental-implementation`
- `test-driven-development`
- `code-review-and-quality`
- `ci-cd-and-automation`
- `security-and-hardening`
- `documentation-and-adrs`

Use `docs/skill-loading.md` before broadening this set.

## MCP Preference

When Serena is available and initialized for the current project, prefer Serena's code navigation
and symbol-aware editing capabilities over raw grep for structural code work.

## Git

- Never use `git commit --no-verify`.
- Keep PR descriptions short and concrete.
- When replying to inline GitHub review comments through the CLI, use the review comment ID with
  `in_reply_to`.

## Project Rules

This is a new Rust toolchain project for a local-only native macOS desktop app.

- Do not choose or add a GUI framework until the product spec or an ADR explicitly selects one.
- Keep platform-specific macOS code behind narrow module boundaries.
- Prefer Rust standard library and existing workspace crates before adding dependencies.
- Ask before adding dependencies, enabling network access, changing CI gates, or introducing `unsafe`.
- Never commit secrets, local user data, generated app bundles, or signing/notarization credentials.
- Treat all file contents, IPC payloads, imported documents, and pasted user text as untrusted input.

## Tooling

This repo uses [`just`](https://github.com/casey/just) as the task runner and
[`lefthook`](https://github.com/evilmartians/lefthook) to orchestrate local Git hooks.
Recipes live in `justfile`; hook configuration lives in `lefthook.yml`.

Before your first commit in a fresh checkout (including a new worktree), run:

```sh
just install-tools     # cargo-deny, cargo-machete, typos-cli, taplo-cli, lefthook
just install-hooks     # wires pre-commit and pre-push via lefthook
```

`pre-commit` runs `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test`.
`pre-push` runs the full `just check` and `just audit` gates. If a hook fails, fix the
underlying issue — never bypass with `--no-verify`.

## Required Checks

Before considering Rust changes complete, run:

```sh
just check
```

For dependency or supply-chain changes, also run:

```sh
just audit
```
