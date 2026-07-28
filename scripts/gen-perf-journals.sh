#!/usr/bin/env bash
# Generate the synthetic performance corpus into target/perf/ and VALIDATE each
# file with the real hledger CLI. A benchmark over a journal the engine would
# reject is worthless, so nothing here is considered generated until
# `hledger check -s` accepts it.
#
# Run inside the nix dev shell (sibling of scripts/gen-corpus.sh):
#     nix develop -c ./scripts/gen-perf-journals.sh            # 5k + 50k
#     nix develop -c ./scripts/gen-perf-journals.sh 5000 50000 200000
#
# The generator (crates/ledgeline-core/benches/corpus.rs) is deterministic: the
# same size always produces a byte-identical file, so benchmark numbers are
# comparable across machines and days. Output is gitignored (`/target/`) —
# 26 MB of synthetic journal does not belong in git.
#
# NOTE: every hledger invocation passes -f explicitly. A bare `hledger` would
# resolve $LEDGER_FILE and read the user's real private journal.
set -euo pipefail

cd "$(dirname "$0")/.."

SIZES=("$@")
if [ ${#SIZES[@]} -eq 0 ]; then
    SIZES=(5000 50000)
fi

echo "building the generator (release)…"
cargo build --release -p ledgeline-core --example gen_journal

# Regenerate unconditionally: the point of the script is to prove the CURRENT
# generator's output validates, not to trust a cache.
rm -f target/perf/*.journal
./target/release/examples/gen_journal "${SIZES[@]}"

echo
for size in "${SIZES[@]}"; do
    file=$(ls target/perf/synthetic-v*-"$size".journal)
    if ! hledger -f "$file" check -s; then
        echo "ERROR: hledger rejected $file" >&2
        exit 1
    fi
    printf '%-40s strict check OK  (%s)\n' "$(basename "$file")" \
        "$(hledger -f "$file" stats | awk -F': *' '/^Txns +:/ {print $2}')"
done

echo
echo "corpus validated with $(hledger --version)"
