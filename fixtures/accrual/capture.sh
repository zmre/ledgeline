#!/usr/bin/env bash
# Capture hledger's own view of the accrual fixture into expected/.
# Never regenerated from Rust: these are what the parity tests compare
# Ledgeline against. Run from anywhere, inside the dev shell:
#   nix develop -c fixtures/accrual/capture.sh
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p expected
ids=$(grep -o 'recognition: [a-z0-9]*' recognition.journal | awk '{print $2}' | sort -u)

# Posted only: plain reports ignore the `~` rules (fact 9).
hledger -f books.journal bal --flat -O csv > expected/bal.csv
hledger -f books.journal bal tag:recognized --flat -O csv > expected/posted.csv

# Each schedule, every occurrence: an explicit past forecast start regenerates
# them all (fact 7), and `tag:_generated-transaction` keeps only those.
for id in $ids; do
  hledger -f books.journal -f recognition.journal print \
    --forecast=2025-01-01..2029-01-01 "tag:recognition=^${id}\$" tag:_generated-transaction \
    > "expected/forecast-${id}.journal"
done

# The schedule ahead, without double-counting today (fact 7).
hledger -f books.journal -f recognition.journal reg tag:recognition \
  --forecast=tomorrow.. --today 2026-11-17 > expected/ahead.txt
