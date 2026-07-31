# Sideromelane

**This repo is archived! Was a fun attempt to make a markdown folder reader/writer/organizer, but I'm never going to finish it!**

Sideromelane is a native macOS Markdown notes app written in Rust. It opens a local folder of
`.md` files, indexes their content and links, and lets you read, edit, search, and explore the
graph of connections — entirely offline. No cloud sync, no proprietary storage format, no plugin
ecosystem; the folder stays usable in any other editor.

The product spec lives in `SPEC.md`; architectural decisions in `docs/adr/`; per-feature specs
in `docs/specs/`. Use `docs/skill-loading.md` to pick up the right agent-skills workflow for
the phase of work.

## Bootstrap

Install the host tooling stack:

```sh
brew bundle
```

If Rust is not already installed, initialize it through the Homebrew-installed Rust toolchain
installer:

```sh
$(brew --prefix rustup)/bin/rustup-init -y
```

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
