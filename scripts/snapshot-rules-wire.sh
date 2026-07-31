#!/usr/bin/env bash
# Snapshot the CSV import-rules wire into fixtures/rules/golden/.
#
# The native /api/* wire has no schema codegen, and neither does this one: the
# `Wire*` structs in crates/ledgeline-server/src/rules_api.rs are mirrored by
# hand on the SPA side. Renaming a Rust field compiles, typechecks and passes
# every test on both sides while the UI quietly renders nothing (CLEANUP.md
# DRY-3). These committed bodies close that: rules_endpoints.rs replays each URI
# and compares BYTES.
#
# REGENERATE ONLY WHEN THE WIRE CONTRACT CHANGED ON PURPOSE, and review the
# diff. An unexplained change here is the bug this file exists to catch.
#
# Run via `just snapshot-rules-wire`, or directly in the nix shell.
set -euo pipefail

cd "$(dirname "$0")/.."

port="${1:-5079}"
out=fixtures/rules/golden
# The journal whose OWN DIRECTORY is the scan root. The whole `tree/` fixture is
# what these bodies describe.
journal=fixtures/rules/tree/main.journal

# A pinned token, not a random one: the snapshot must be reproducible, and this
# server only ever sees a committed fixture on loopback.
export LEDGELINE_TOKEN=ledgeline-rules-fixture-token

cargo build -p ledgeline-server
./target/debug/ledgeline --server "$journal" --port "$port" &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT

# /version is token-gated too, so the readiness probe has to authenticate.
auth=(-H "Authorization: Bearer $LEDGELINE_TOKEN")
for _ in $(seq 1 60); do
    curl -fsS "${auth[@]}" "http://127.0.0.1:$port/version" >/dev/null 2>&1 && break
    sleep 0.5
done
# Fail loudly if it never came up, rather than writing empty goldens.
curl -fsS "${auth[@]}" "http://127.0.0.1:$port/version" >/dev/null

count=0
while IFS=$'\t' read -r name uri; do
    case "$name" in '' | '#'*) continue ;; esac
    # No trailing newline and no reformatting: these are the RAW response bytes,
    # so the Rust golden test can compare them byte-for-byte (matching the
    # fixtures/native/v1 convention).
    curl -fsS "${auth[@]}" "http://127.0.0.1:$port$uri" > "$out/$name.json"
    count=$((count + 1))
done < "$out/requests.tsv"
echo "snapshotted $count rules endpoints into $out"
