#!/usr/bin/env bash
# Assert that every import-rules fixture is a rules file REAL hledger accepts.
#
# Why this exists: the Rust round-trip tests prove we do not damage what we did
# not touch. They say nothing about whether the fixtures are realistic — a
# fixture containing invalid syntax would round-trip perfectly and prove
# nothing. This is the check that keeps the corpus honest. (An hledger upgrade
# tightening the grammar shows up here, loudly, instead of as a user's file
# silently failing to import.)
#
# Deliberately fixture-only. We NEVER run hledger against a user's rules file:
# a `source ... | CMD` rule is a shell command hledger executes.
#
# Run via `just rules-check`, or directly in the nix shell.
set -euo pipefail

cd "$(dirname "$0")/.."

RULES=fixtures/rules
status=0

# Data-backed fixtures: driven from their CSV, which is how hledger finds the
# sibling `FILE.csv.rules`. (`-f FILE.rules` is NOT equivalent — since 1.50 it
# demands an explicit `source` rule.)
#
# `tree/` is a DISCOVERY fixture, not a syntax one — its shape is the point, and
# most of its files are decoys the scan must refuse to find. Only the one file
# discovery is supposed to return is checked here, because that one is also a
# rules file the Rust suites parse and summarize, so it has to be real.
for csv in "$RULES"/simple/*.csv "$RULES"/advanced/*.csv "$RULES"/tree/import/2026/*.csv; do
    printf '%-46s ' "$csv"
    if out=$(hledger -f "$csv" print 2>&1); then
        echo "OK"
    else
        echo "FAIL"
        echo "$out" | sed 's/^/    /'
        status=1
    fi
done

# Edge-encoding fixtures have no data file of their own (they are named
# `*.rules`, not `*.csv.rules`), so pair them with a shared ISO-dated CSV via
# --rules. They exist to pin byte preservation across BOM/CRLF/EOF oddities;
# their syntax still has to be valid.
edge_csv=$(mktemp -t ledgeline-rules-edge.XXXXXX.csv)
trap 'rm -f "$edge_csv"' EXIT
printf 'Date,Description,Amount\n2024-01-15,COFFEE HOUSE,-6.45\n2024-01-16,RENT,-1850.00\n' > "$edge_csv"

# empty.rules and only-comments.rules are EXCLUDED on purpose: a rules file with
# no `date` field is not a valid rules file, and that is exactly what they are
# for — proving Ledgeline still opens, shows and byte-preserves a file hledger
# would refuse. Asserting hledger accepts them would assert the opposite of
# their purpose.
for rules in "$RULES"/edge/*.rules; do
    case "$(basename "$rules")" in
        empty.rules | only-comments.rules) continue ;;
    esac
    printf '%-46s ' "$rules"
    if out=$(hledger -f "$edge_csv" --rules "$rules" print 2>&1); then
        echo "OK"
    else
        echo "FAIL"
        echo "$out" | sed 's/^/    /'
        status=1
    fi
done

# --- fixtures/import/match — the rules-MATCHING corpus (WP-11) ---------------
#
# Same house rule, different point. Here the corpus is two rules files that are
# right and three that are wrong in three different ways, and "wrong" must still
# mean "a real rules file hledger accepts" — otherwise the matcher would be
# scored against syntax errors rather than against fact 4's silent garbage.
#
# See fixtures/import/match/README.md.
MATCH=fixtures/import/match

check() {
    printf '%-46s ' "$3"
    if out=$(hledger -f "$1" --rules "$2" print 2>&1); then
        echo "OK"
    else
        echo "FAIL"
        echo "$out" | sed 's/^/    /'
        status=1
    fi
}

# The two correct pairs, driven from their own CSV.
for csv in "$MATCH"/checking.csv "$MATCH"/creditcard.csv; do
    check "$csv" "$csv.rules" "$csv"
done

# The two FACT 4 fixtures. Both must exit 0 against checking.csv — that IS the
# point of them. hledger reads them happily and produces amountless postings and
# bare-commodity amounts; only the structured output shows it, which is what
# crates/ledgeline-core/tests/matching.rs asserts.
for rules in "$MATCH"/garbage-success.rules "$MATCH"/no-currency.rules; do
    check "$MATCH/checking.csv" "$rules" "$rules"
done

# wrong-dateformat.rules is driven from the data it was WRITTEN for, because it
# is a genuine rules file for a German bank export. Against checking.csv it
# fails — and reaching that conclusion WITHOUT running hledger is exactly what
# stage 1 of the matcher exists to do.
match_csv=$(mktemp -t ledgeline-match-euro.XXXXXX.csv)
trap 'rm -f "$edge_csv" "$match_csv"' EXIT
printf 'Datum,Beschreibung,Soll,Haben\n15.01.2024,GEHALT,,3000.00\n16.01.2024,SUPERMARKT,45.20,\n' > "$match_csv"
check "$match_csv" "$MATCH/wrong-dateformat.rules" "$MATCH/wrong-dateformat.rules"

if [ "$status" -ne 0 ]; then
    echo
    echo "One or more rules fixtures are not valid hledger rules files." >&2
fi
exit "$status"
