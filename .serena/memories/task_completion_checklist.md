# Task Completion Checklist

After every commit:
1. `just check` must pass clean (fmt-check + clippy + test + doc)
2. `just audit` must pass clean (deny + machete + typos + taplo)
3. Test count must not decrease relative to the prior commit
4. Each AC in the active spec must have at least one test per the spec's Testing Strategy section
