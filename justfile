set shell := ["zsh", "-cu"]

cargo_bin := env_var("HOME") + "/.cargo/bin"
export PATH := cargo_bin + ":" + env_var("PATH")

profile := "default"

default:
    @just --list

link-skills profile=profile:
    ./scripts/link-skills.sh {{ profile }}

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features

doc:
    cargo doc --workspace --all-features --no-deps

check: fmt-check lint test doc

package:
    ./scripts/package-macos-app.sh

audit:
    cargo deny check
    cargo machete
    typos
    taplo fmt --check

install-tools:
    cargo install cargo-deny cargo-machete typos-cli taplo-cli just
    if (( ! $+commands[lefthook] )); then brew install lefthook; fi

install-hooks:
    lefthook install

hooks:
    lefthook run pre-commit --force --all-files
