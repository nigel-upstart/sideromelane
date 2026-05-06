# ADR 0005: Local macOS Packaging

## Status

Accepted

## Context

The app needs to be packageable and shippable locally, but signing and notarization are not yet
required. The repository must not commit generated app bundles or signing credentials.

## Decision

Add a local packaging command that builds `sideromelane-app` in release mode and assembles an
unsigned `.app` bundle under `target/package/`.

The package contains:

- `Contents/Info.plist`
- `Contents/MacOS/sideromelane`
- `Contents/Resources/`

Signing and notarization remain out of scope until a release/distribution ADR selects certificate
handling and credential storage.

## Consequences

- Developers can produce a local macOS app bundle without external services.
- Packaging remains transparent and scriptable.
- Generated bundles stay out of version control.
- Gatekeeper warnings are expected for unsigned local builds.
