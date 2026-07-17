#!/usr/bin/env bash
# Regenerate the hermetic parse-corpus goldens under fixtures/corpus/ from the
# real hledger CLI. The corpus is a curated set of journal snippets adapted from
# hledger's own parse tests (hledger/test/journal/*.test); the committed
# *.print.json goldens let crates/ledgeline-core/tests/corpus.rs run without
# hledger installed. Rerunning after an hledger upgrade produces a reviewable
# diff. Run directly in the nix shell (sibling of scripts/gen-golden.sh).
#
# For each fixtures/corpus/*.journal (a journal hledger ACCEPTS): capture
#   hledger -f FILE print -O json  ->  FILE-without-.journal + .print.json
# For each fixtures/corpus/errors/*.journal (a journal hledger REJECTS): confirm
# `hledger print` exits non-zero and drop a .expect-error marker beside it.
set -euo pipefail

cd "$(dirname "$0")/.."

CORPUS=fixtures/corpus
ERRORS=$CORPUS/errors

accepted=0
for f in "$CORPUS"/*.journal; do
    out="${f%.journal}.print.json"
    if ! hledger -f "$f" print -O json > "$out"; then
        echo "ERROR: hledger rejected $f, but it is a corpus (accepted) fixture" >&2
        rm -f "$out"
        exit 1
    fi
    accepted=$((accepted + 1))
done

rejected=0
for f in "$ERRORS"/*.journal; do
    if hledger -f "$f" print -O json > /dev/null 2>&1; then
        echo "ERROR: hledger accepted $f, but it is an error (rejected) fixture" >&2
        exit 1
    fi
    : > "${f%.journal}.expect-error"
    rejected=$((rejected + 1))
done

echo "regenerated $accepted print.json goldens in $CORPUS ($(hledger --version))"
echo "confirmed $rejected rejected fixtures in $ERRORS"
