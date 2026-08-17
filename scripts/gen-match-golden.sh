#!/usr/bin/env bash
# Regenerate the rules-matching goldens under fixtures/import/match/golden/.
#
# Why goldens at all: `matching::signals_from_hledger_json` is pure, and the only
# authority on what hledger's `print -O json` actually looks like is hledger. A
# hand-written JSON blob would let us test our reading of a shape we invented.
# These are the real bytes, committed, so `cargo test` stays HERMETIC — no test
# needs hledger on PATH — while still being anchored to the binary.
#
# Run from the nix dev shell. Rerun after an hledger upgrade and REVIEW THE DIFF:
# a change here is a change in the contract stage 2 reads.
#
# Mirrors scripts/gen-budget-golden.sh: fixed inputs and no dates from "today",
# so rerunning produces a reviewable diff rather than churn.
#
# One post-step is unavoidable. hledger ABSOLUTIZES the `sourceName` it embeds in
# `tsourcepos`, so the raw output carries whoever generated it's home directory.
# That is committed path disclosure, and it also makes the bytes machine-specific.
# The repo-root prefix is therefore stripped, leaving a relative
# `fixtures/import/match/checking.csv`. Safe to rewrite: `sourceName` is the one
# field of this JSON that matching.rs is forbidden to read (docs/imports.md
# § Security — errors never disclose paths), so nothing under test can notice.
set -euo pipefail

cd "$(dirname "$0")/.."

DIR=fixtures/import/match
OUT=$DIR/golden
mkdir -p "$OUT"

# stem|data|rules — the four combinations stage 2 is scored against. The last two
# are FACT 4: hledger exits 0 on both and produces unusable output.
cases=(
  "checking|$DIR/checking.csv|$DIR/checking.csv.rules"
  "creditcard|$DIR/creditcard.csv|$DIR/creditcard.csv.rules"
  "garbage-success|$DIR/checking.csv|$DIR/garbage-success.rules"
  "no-currency|$DIR/checking.csv|$DIR/no-currency.rules"
)

root=$PWD

for case in "${cases[@]}"; do
  IFS='|' read -r stem data rules <<<"$case"
  # `|` as the sed delimiter so the path's own slashes need no escaping.
  hledger print -f "$data" --rules "$rules" -O json |
    sed "s|$root/||g" >"$OUT/$stem.print.json"
  if grep -q "$root" "$OUT/$stem.print.json"; then
    echo "refusing to commit an absolute path in $stem.print.json" >&2
    exit 1
  fi
  printf '%-18s %s\n' "$stem" "OK"
done

hledger --version >"$OUT/HLEDGER_VERSION"

echo "regenerated $(find "$OUT" -name '*.print.json' | wc -l | tr -d ' ') matching goldens in $OUT ($(cat "$OUT/HLEDGER_VERSION"))"
