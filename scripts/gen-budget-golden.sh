#!/usr/bin/env bash
# Regenerate the budget-report goldens under fixtures/budget/ from the
# fixtures/budget/*.journal sources via the hledger CLI. Mirrors gen-golden.sh:
# goldens are committed, all dates are FIXED constants (never "today") so the
# `hledger bal -M --budget -O json` output is deterministic, and rerunning after
# an hledger upgrade yields a reviewable diff. The matching `.txt` capture is
# committed alongside each JSON purely as a human-readable reference. Run in the
# nix dev shell.
set -euo pipefail

cd "$(dirname "$0")/.."

DIR=fixtures/budget

# name|args|out-stem — one budget report each. `args` are the hledger flags
# (fixed -b/-e span, plus any --budget=DESCPAT). The Rust golden test
# (tests/budget_golden.rs) runs budget_report with matching parameters.
cases=(
  "basic|-b 2026-01-01 -e 2026-03-01 --budget|basic"
  "parents|-b 2026-01-01 -e 2026-02-01 --budget|parents"
  "unbudgeted|-b 2026-01-01 -e 2026-02-01 --budget|unbudgeted"
  "descpat|-b 2026-01-01 -e 2026-02-01 --budget=housing|descpat-housing"
  "descpat|-b 2026-01-01 -e 2026-02-01 --budget=grocer|descpat-grocer"
  "weekly|-b 2026-01-01 -e 2026-02-01 --budget|weekly"
)

for case in "${cases[@]}"; do
  IFS='|' read -r name args stem <<<"$case"
  # shellcheck disable=SC2086 # word-splitting of $args is intentional
  hledger -f "$DIR/$name.journal" bal -M $args -O json >"$DIR/$stem.budget.json"
  # shellcheck disable=SC2086
  hledger -f "$DIR/$name.journal" bal -M $args >"$DIR/$stem.budget.txt"
done

hledger --version >"$DIR/HLEDGER_VERSION"

echo "regenerated $(ls "$DIR"/*.budget.json | wc -l | tr -d ' ') budget goldens in $DIR ($(cat "$DIR/HLEDGER_VERSION"))"
