# Suggested Commands

## Development workflow
```
just check          # fmt-check + clippy + test + doc — run before every commit
just audit          # cargo deny + machete + typos + taplo — run before PR
just fmt            # auto-format all code
just test           # cargo test --workspace --all-features
just package        # build macOS .app bundle
cargo run -p sideromelane-app --release   # run the app
```

## Task completion checklist
1. `just check` must pass clean
2. `just audit` must pass clean
3. Test count must not decrease relative to prior commit
