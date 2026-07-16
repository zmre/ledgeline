#!/usr/bin/env bash
# Snapshot the hledger-web JSON API responses for fixtures/sample.journal
# into fixtures/api/vVERSION/ (directory named from the /version response).
# These raw snapshots are the normalizer's regression net (see WP-02/WP-09).
# Launches its own hledger-web on $PORT (default 5077) and kills it on exit.
set -euo pipefail

cd "$(dirname "$0")/.."

JOURNAL=fixtures/sample.journal
PORT="${PORT:-5077}"
HOST=127.0.0.1
BASE="http://$HOST:$PORT"

hledger-web -f "$JOURNAL" --serve-api --cors='*' --allow=view --host "$HOST" --port "$PORT" &
PID=$!
trap 'kill "$PID" 2>/dev/null || true' EXIT

# Wait for the server to come up (max ~10s).
for _ in $(seq 1 50); do
    if curl -fsS "$BASE/version" >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done
curl -fsS "$BASE/version" >/dev/null # fail loudly if it never came up

# /version returns a JSON string like "1.52"; sanitize into a directory name.
VERSION=$(curl -fsS "$BASE/version" | tr -d '"' | cut -d',' -f1 | tr -d ' ')
OUT="fixtures/api/v$VERSION"
mkdir -p "$OUT"

for ep in version transactions accountnames prices commodities accounts; do
    curl -fsS "$BASE/$ep" > "$OUT/$ep.json"
done

echo "snapshotted 6 endpoints into $OUT"
