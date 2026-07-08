#!/usr/bin/env bash
# Regenerate fixtures/golden/ from fixtures/sample.journal via the hledger CLI.
# Goldens are committed; rerunning after an hledger upgrade produces a
# reviewable diff. All dates are FIXED constants (never "today") so the
# output is deterministic. Run via `just golden` or directly in the nix shell.
set -euo pipefail

cd "$(dirname "$0")/.."

JOURNAL=fixtures/sample.journal
OUT=fixtures/golden
mkdir -p "$OUT"

hledger -f "$JOURNAL" bs -O json --depth 1 -e 2026-07-01 > "$OUT/bs-d1.json"
hledger -f "$JOURNAL" bs -O json --depth 3 -e 2026-07-01 > "$OUT/bs-d3.json"
hledger -f "$JOURNAL" is -O json -b 2026-01-01 -e 2026-07-01 --depth 2 > "$OUT/is-d2.json"
hledger -f "$JOURNAL" cashflow -O json -M -b 2026-01-01 -e 2026-07-01 > "$OUT/cf-monthly.json"
hledger -f "$JOURNAL" bal -O json --value=end,'$' -e 2026-07-01 assets liabilities > "$OUT/networth-spot.json"
hledger --version > "$OUT/HLEDGER_VERSION"

echo "regenerated $(ls "$OUT" | wc -l | tr -d ' ') files in $OUT ($(cat "$OUT/HLEDGER_VERSION"))"
