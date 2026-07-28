# Ledgeline task runner. Run `just --list` for an overview.

# Start the SvelteKit dev server. The SPA talks to the in-process Ledgeline
# engine, so run the engine alongside it (e.g. `just serve-engine`) in another shell.
dev:
    cd web && bun run dev

# Run unit tests (vitest)
test:
    cd web && bun run test:unit

# Without LEDGELINE_API_URL the *.integration.test.ts suites report as SKIPPED, so
# plain `just test` never exercises the fixture→engine→JSON→decode→rendered-string
# path at all. Local twin of the CI `e2e` job's first half.
# Run the round-trip contract tests against a live engine (vitest)
test-integration port="5055":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p ledgeline-server
    ./target/debug/ledgeline --server fixtures/sample.journal --port {{port}} &
    server=$!
    trap 'kill "$server" 2>/dev/null || true' EXIT
    for _ in $(seq 1 60); do
        curl -fsS http://127.0.0.1:{{port}}/version >/dev/null 2>&1 && break
        sleep 0.5
    done
    curl -fsS http://127.0.0.1:{{port}}/version
    cd web && LEDGELINE_API_URL=http://127.0.0.1:{{port}} bun run test:unit

# playwright.config.ts launches ../target/debug/ledgeline as the fixture API
# server, so build it first ($LEDGELINE_BIN overrides that path, as CI does to
# point at the Nix-built binary).
# Run e2e tests (playwright)
e2e:
    cargo build -p ledgeline-server
    cd web && bun run test:e2e

# Typecheck + svelte-check
check:
    cd web && bun run check

# Regenerate golden report fixtures from fixtures/sample.journal via hledger CLI
golden:
    ./scripts/gen-golden.sh

# Snapshot raw hledger-web JSON API responses into fixtures/api/vVERSION/
snapshot-api:
    ./scripts/snapshot-api.sh

# Production build (static SPA)
build:
    cd web && bun run build

# Build the macOS app bundle (Ledgeline.app) with the real SPA embedded, via Nix.
# The SPA is built inside Nix, so no prior `just build` is needed.
package-mac:
    nix build .#macApp --accept-flake-config -o result-macapp
    mkdir -p dist
    cp -RL result-macapp/Applications/Ledgeline.app dist/Ledgeline.app
    chmod -R u+w dist/Ledgeline.app
    @echo "Built dist/Ledgeline.app — run it with: open dist/Ledgeline.app"

# --- Rust engine (crates/) ---

# Build the Rust journal engine
engine-build:
    cargo build

# Test the Rust journal engine
engine-test:
    cargo test

# Format + lint the Rust engine (clippy warnings are errors)
engine-check:
    cargo fmt --check && cargo clippy --all-targets -- -D warnings

# Run the local engine server (Phase 2+): `just serve-engine ~/finance/2026.journal`
serve-engine file="fixtures/sample.journal":
    cargo run -p ledgeline-server -- {{file}}

# --- Performance (see docs/perf-baseline.md) ---

# Generate the deterministic synthetic corpus into target/perf/ and validate it
# with `hledger check -s`. Sizes default to 5000 50000; pass your own.
perf-corpus *sizes:
    ./scripts/gen-perf-journals.sh {{sizes}}

# Criterion benches over the 5k + 50k corpus. Optional filter, e.g.
# `just bench reports/net_worth`. Missing corpora are generated on first use.
bench filter="":
    cargo bench -p ledgeline-core -- {{filter}}

# Same, plus the 200k corpus CLEANUP.md's Phase 6 table was measured on (~30 min).
bench-big filter="":
    LEDGELINE_BENCH_SIZES=5000,50000,200000 cargo bench -p ledgeline-core -- {{filter}}

# Record a named criterion baseline (all three sizes) to diff a later run against.
bench-save name="baseline":
    LEDGELINE_BENCH_SIZES=5000,50000,200000 cargo bench -p ledgeline-core -- --save-baseline {{name}}

# Re-run all three sizes and report the change against a saved baseline.
bench-compare name="baseline":
    LEDGELINE_BENCH_SIZES=5000,50000,200000 cargo bench -p ledgeline-core -- --baseline {{name}}

# Peak RSS of holding one journal's Snapshot, stage by stage. macOS only
# (`/usr/bin/time -l`); on Linux swap in `/usr/bin/time -v`.
perf-rss size="200000":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p ledgeline-core --example load_rss
    for stage in text parse clone value snapshot bytes; do
        peak=$(/usr/bin/time -l ./target/release/examples/load_rss {{size}} --stage "$stage" 2>&1 \
            | awk '/maximum resident set size/ {print $1}')
        awk -v s="$stage" -v p="$peak" 'BEGIN{printf "%-10s peak RSS %9.1f MB\n", s, p/1048576}'
    done
