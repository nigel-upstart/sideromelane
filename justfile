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

# Build and zip the .app for a release — mirrors what the CI release workflow does.
# Usage: just zip-release VERSION=0.2.0
zip-release VERSION="0.0.0-dev":
    ./scripts/package-macos-app.sh
    /usr/libexec/PlistBuddy \
        -c "Set :CFBundleShortVersionString {{VERSION}}" \
        -c "Set :CFBundleVersion {{VERSION}}" \
        target/package/Sideromelane.app/Contents/Info.plist
    cd target/package && ditto -c -k --sequesterRsrc --keepParent \
        Sideromelane.app \
        "Sideromelane-{{VERSION}}-macos.zip"
    @echo "target/package/Sideromelane-{{VERSION}}-macos.zip"

audit:
    cargo deny check
    cargo machete
    typos
    taplo fmt --check

install-tools:
    brew bundle
    cargo install cargo-machete

install-hooks:
    lefthook install

hooks:
    lefthook run pre-commit --force --all-files
