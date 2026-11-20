#!/usr/bin/env bash
# Capture hledger's own view of the receivables fixture into expected/.
# Never regenerated from Rust: these are what
# crates/ledgeline-core/tests/accrual_library.rs compares Ledgeline's
# receivables against. Run from anywhere, inside the dev shell:
#   nix develop -c fixtures/receivables/capture.sh
set -euo pipefail
# Every command names its journal with -f; never fall back to a real one.
unset LEDGER_FILE
cd "$(dirname "$0")"
mkdir -p expected

hledger -s -f books.journal check
hledger -s -f cash.journal check

# Accrual books: everything, the receivable (account tag kpi: inherited by
# its postings), what no settlement link covers, and each invoice's own
# transactions — the invoice, its payments and its void all carry invoice:.
hledger -f books.journal bal --flat -O csv > expected/bal.csv
hledger -f books.journal bal tag:kpi=receivable --flat -O csv > expected/receivable.csv
hledger -f books.journal bal tag:kpi=receivable not:tag:match --flat -O csv > expected/unlinked.csv
for n in 101 102 103 104 105 106; do
  hledger -f books.journal bal "tag:invoice=^${n}\$" --flat -O csv > "expected/invoice-${n}.csv"
done

# Cash books: each applied deposit, re-split into the invoice's rows.
for n in 201 202; do
  hledger -f cash.journal bal "tag:invoice=^${n}\$" --flat -O csv > "expected/cash-invoice-${n}.csv"
done
