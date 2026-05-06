#!/usr/bin/env zsh
set -euo pipefail

profile="${1:-default}"
source_root="${AGENTS_SKILLS_HOME:-$HOME/.agents/skills}"
dest_root="${REPO_SKILLS_HOME:-skills}"

default_skills=(
  using-agent-skills
  spec-driven-development
  planning-and-task-breakdown
  incremental-implementation
  test-driven-development
  code-review-and-quality
  ci-cd-and-automation
  security-and-hardening
  documentation-and-adrs
)

on_demand_skills=(
  api-and-interface-design
  browser-testing-with-devtools
  code-simplification
  context-engineering
  debugging-and-error-recovery
  deprecation-and-migration
  frontend-ui-engineering
  git-workflow-and-versioning
  idea-refine
  performance-optimization
  shipping-and-launch
  source-driven-development
)

case "$profile" in
  default)
    skills=("${default_skills[@]}")
    ;;
  all)
    skills=("${default_skills[@]}" "${on_demand_skills[@]}")
    ;;
  *)
    echo "Unknown profile: $profile" >&2
    echo "Usage: $0 [default|all]" >&2
    exit 2
    ;;
esac

mkdir -p "$dest_root"

for skill in "${skills[@]}"; do
  src="$source_root/$skill"
  dest="$dest_root/$skill"
  if [[ ! -f "$src/SKILL.md" ]]; then
    echo "Missing $src/SKILL.md. Run: npx skills add addyosmani/agent-skills --yes --global" >&2
    exit 1
  fi
  mkdir -p "$dest"
  rsync -a "$src/" "$dest/"
  echo "linked $skill"
done

cat > "$dest_root/README.md" <<'EOF'
# Skills

Imported from `~/.agents/skills` by `just link-skills`.

Default profile:

- using-agent-skills
- spec-driven-development
- planning-and-task-breakdown
- incremental-implementation
- test-driven-development
- code-review-and-quality
- ci-cd-and-automation
- security-and-hardening
- documentation-and-adrs

Use `just link-skills all` only when the additional upstream skills should be exposed in
this repo.
EOF
