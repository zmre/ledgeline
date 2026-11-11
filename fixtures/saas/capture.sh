#!/usr/bin/env bash
# Capture hledger's own figures for startup.journal into expected/.
#
# These CSVs are the parity oracle for crates/ledgeline-core/tests/saas_metrics.rs.
# They come from hledger (1.52), never from Ledgeline: re-run this script after
# regenerating startup.journal, and commit both together.
#
# The commands double as the "Check it with hledger" list in
# docs/reports/startup-metrics.md, so the docs and the tests cannot drift.
set -euo pipefail
cd "$(dirname "$0")"

J=startup.journal
SPAN=(-b 2025-01 -e 2026-07)
mkdir -p expected

# MRR per customer: recognised recurring revenue pivoted on the customer: tag.
hledger -f "$J" bal tag:kpi=recurring --pivot customer -M --invert -O csv "${SPAN[@]}" > expected/customer-mrr.csv
# MRR: the same, without the pivot (the Total: row).
hledger -f "$J" bal tag:kpi=recurring --depth 1 -M --invert -O csv "${SPAN[@]}" > expected/mrr.csv
# MRR per segment (the segment: account tag, inherited by the leaves).
hledger -f "$J" bal tag:kpi=recurring --pivot segment -M --invert -O csv "${SPAN[@]}" > expected/segment-mrr.csv
# MRR per class: a posting's own class: wins over its account's.
hledger -f "$J" bal tag:kpi=recurring --pivot class -M --invert -O csv "${SPAN[@]}" > expected/class-mrr.csv
# Deferred revenue balance at each month end (positive = owed).
hledger -f "$J" bal tag:kpi=deferred-revenue -M -H --invert -O csv "${SPAN[@]}" > expected/deferred.csv
# Billings: revenue plus the change in deferred revenue.
hledger -f "$J" bal "expr:type:R or tag:kpi=deferred-revenue" -M --invert -O csv "${SPAN[@]}" > expected/billings.csv
# Cost of revenue (Ledgeline's issection: tag).
hledger -f "$J" bal tag:issection=cogs -M -O csv "${SPAN[@]}" > expected/cogs.csv
# Cash at each month end.
hledger -f "$J" bal type:C -M -H -O csv "${SPAN[@]}" > expected/cash.csv
