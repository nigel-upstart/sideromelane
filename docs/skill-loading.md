# Skill Loading

This repo uses `addyosmani/agent-skills` as workflow guidance, but does not load every skill by
default. The upstream guidance recommends selective loading to avoid wasting context.

## Default Profile

Import these with:

```sh
just link-skills
```

- `using-agent-skills`: route work to the right workflow.
- `spec-driven-development`: required before building the app.
- `planning-and-task-breakdown`: turn approved specs into small tasks.
- `incremental-implementation`: build one verifiable slice at a time.
- `test-driven-development`: prove behavior with tests.
- `code-review-and-quality`: review every substantive change.
- `ci-cd-and-automation`: maintain automated quality gates.
- `security-and-hardening`: apply to local data, file input, IPC, and dependencies.
- `documentation-and-adrs`: document framework and architecture decisions.

## On Demand

Use these only when the task calls for them:

- `frontend-ui-engineering`: after a GUI framework is selected.
- `browser-testing-with-devtools`: only for a WebView/browser-based app surface.
- `api-and-interface-design`: if an IPC, plugin, or local service boundary is introduced.
- `performance-optimization`: after product-level performance targets exist.
- `debugging-and-error-recovery`: when fixing a reproduced bug.
- `deprecation-and-migration`: when replacing established code or data formats.
- `shipping-and-launch`: when packaging, signing, notarizing, or distributing the app.
- `source-driven-development`: when implementing against official framework or macOS docs.
- `code-simplification`: during cleanup passes after behavior is covered.
- `git-workflow-and-versioning`: when shaping commits or PRs.
- `idea-refine`: when the product concept is still too vague to spec.
- `context-engineering`: when the repo becomes large enough that context selection matters.

## Importing All Skills

If the repo should expose every upstream skill, run:

```sh
just link-skills all
```

Do this deliberately. Availability is not the same as activation; agents should still invoke only
the workflow required by the current task.
