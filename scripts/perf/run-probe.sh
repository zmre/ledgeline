#!/usr/bin/env bash
# Launch the desktop app on a journal with the perf probe injected, print every
# PERF line, and sample memory while the probe holds the app open after its
# scroll test. macOS only (uses `footprint`).
#
#   cargo build --release -p ledgeline
#   cargo run --release -p ledgeline-core --example gen_journal -- 200000
#   scripts/perf/run-probe.sh target/perf/synthetic-v1-200000.journal
#
# HOME is pointed at a scratch directory so the run never touches your real
# preferences or recent-journals list. See docs/development/perf-baseline.md.
set -euo pipefail
journal=${1:?usage: run-probe.sh <journal>}
bin=${LEDGELINE_BIN:-target/release/ledgeline}
probe=$(cd "$(dirname "$0")" && pwd)/probe.js
scratch=${PERF_HOME:-/tmp/ll-perf/home}
mkdir -p "$scratch"

log=$(mktemp "${TMPDIR:-/tmp}/ll-perf.XXXXXX")
HOME=$scratch LEDGELINE_PERF=$probe "$bin" "$journal" 2>"$log" &
app=$!
trap 'kill $app 2>/dev/null || true; rm -f "$log"' EXIT

sampled=0
while kill -0 $app 2>/dev/null; do
    if [[ $sampled -eq 0 ]] && grep -q '"ev":"scroll_stats"' "$log"; then
        sampled=1
        sleep 2
        echo "MEM ledgeline $(footprint -p $app 2>/dev/null | awk '/phys_footprint:/ {print $2, $3; exit}')"
        # The newest WebContent process is this app's page.
        web=$(pgrep -n -f com.apple.WebKit.WebContent || true)
        [[ -n $web ]] && echo "MEM WebContent $(footprint -p "$web" 2>/dev/null | awk '/phys_footprint:/ {print $2, $3; exit}')"
    fi
    sleep 0.25
done
grep '^PERF' "$log" || cat "$log"
