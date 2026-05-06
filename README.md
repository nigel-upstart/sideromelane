# Sideromelane

Sideromelane is a new Rust toolchain project for a local-only native macOS desktop app.

The app spec is not written yet. Start with `SPEC.md`, then use `docs/skill-loading.md` to load the
right agent-skills workflow for the phase of work.

## Bootstrap

Install the upstream skills globally, then import the repo default set:

```sh
npx skills add addyosmani/agent-skills --yes --global
just link-skills
```

## Development

```sh
just fmt
just lint
just test
just check
```

Build a local unsigned macOS app bundle:

```sh
just package
```

Optional quality tools are documented in `docs/rust-quality.md`.

## Git Hooks

```sh
just install-tools
just install-hooks
```
