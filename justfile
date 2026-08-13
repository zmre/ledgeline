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
    # PINNED, not the random per-process token: the probe below and the vitest
    # suite both have to know it. Every wire and /api route is token-gated, so an
    # unauthenticated probe gets a 401 and `curl -fsS` exits 22.
    export LEDGELINE_TOKEN=ledgeline-integration-token
    cargo build -p ledgeline-server
    ./target/debug/ledgeline --server fixtures/sample.journal --port {{port}} &
    server=$!
    trap 'kill "$server" 2>/dev/null || true' EXIT
    auth=(-H "Authorization: Bearer $LEDGELINE_TOKEN")
    for _ in $(seq 1 60); do
        curl -fsS "${auth[@]}" http://127.0.0.1:{{port}}/version >/dev/null 2>&1 && break
        sleep 0.5
    done
    # Fails loudly if the engine never came up OR the token is wrong — better
    # here than as a confusing wall of 401s inside the test suite.
    curl -fsS "${auth[@]}" http://127.0.0.1:{{port}}/version
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

# Assert every fixtures/rules/ file is a rules file real hledger accepts. The
# Rust round-trip tests only prove we don't damage what we didn't touch; this is
# what keeps the corpus itself honest.
rules-check:
    ./scripts/check-rules-fixtures.sh

# Regenerate the rules-matching goldens in fixtures/import/match/golden/ from
# real `hledger print -O json` runs. Run this ONLY when a match fixture or the
# scoring signals changed on purpose — the committed goldens are what keep
# `cargo test` hermetic (no hledger required to run the scoring tests).
match-golden:
    ./scripts/gen-match-golden.sh

# The hledger-backed checks, all opt-in so `cargo test` stays hermetic. These
# are the only things that prove our output is syntax hledger actually accepts,
# rather than syntax we did not damage. See docs/imports.md.
hledger-checks:
    LEDGELINE_HLEDGER_RENDER_CHECK=1 cargo test -p ledgeline-core --test rules_hledger_render
    LEDGELINE_HLEDGER_MATCH_CHECK=1 cargo test -p ledgeline-core --test matching
    LEDGELINE_HLEDGER_SORT_CHECK=1 cargo test -p ledgeline-core --test sort

# Snapshot raw hledger-web JSON API responses into fixtures/api/vVERSION/
snapshot-api:
    ./scripts/snapshot-api.sh

# The native /api/* wire has no schema codegen: 28 Rust `Wire*` structs are
# mirrored by hand in web/src/lib/api/nativeDecode.ts. Renaming a Rust field
# used to compile, typecheck and pass every test on both sides while the SPA
# quietly rendered $0.00 (CLEANUP.md DRY-3). These committed bodies close that:
# native_wire_golden.rs replays each URI and compares BYTES, and
# nativeDecode.test.ts decodes these same files, so a rename fails on BOTH sides.
# Regenerate ONLY when the wire contract changed on purpose, and review the diff.
# Snapshot raw native /api/* JSON responses into fixtures/native/v1/
snapshot-native port="5078":
    #!/usr/bin/env bash
    set -euo pipefail
    # A pinned token, not a random one: the snapshot must be reproducible, and
    # this server only ever sees fixtures/sample.journal on loopback.
    export LEDGELINE_TOKEN=ledgeline-native-fixture-token
    out=fixtures/native/v1
    cargo build -p ledgeline-server
    ./target/debug/ledgeline --server fixtures/sample.journal --port {{port}} &
    server=$!
    trap 'kill "$server" 2>/dev/null || true' EXIT
    # /version is token-gated too, so the readiness probe has to authenticate.
    auth=(-H "Authorization: Bearer $LEDGELINE_TOKEN")
    for _ in $(seq 1 60); do
        curl -fsS "${auth[@]}" "http://127.0.0.1:{{port}}/version" >/dev/null 2>&1 && break
        sleep 0.5
    done
    curl -fsS "${auth[@]}" "http://127.0.0.1:{{port}}/version" >/dev/null # fail loudly if it never came up
    count=0
    while IFS=$'\t' read -r name uri; do
        case "$name" in ''|'#'*) continue ;; esac
        # No trailing newline and no reformatting: these are the RAW response
        # bytes, so the Rust golden test can compare them byte-for-byte
        # (matching the fixtures/api/v1.52 convention).
        curl -fsS "${auth[@]}" "http://127.0.0.1:{{port}}$uri" > "$out/$name.json"
        count=$((count + 1))
    done < "$out/requests.tsv"
    echo "snapshotted $count native endpoints into $out"

# The CSV import-rules wire has no schema codegen either: the `Wire*` structs in
# rules_api.rs are mirrored by hand on the SPA side, so a renamed Rust field
# compiles and passes on both sides while the imports screen quietly renders
# nothing (the DRY-3 shape). These committed bodies close that:
# rules_endpoints.rs replays each URI over fixtures/rules/tree/ and compares
# BYTES. Regenerate ONLY when the wire contract changed on purpose, and review
# the diff.
# Snapshot raw /api/rules JSON responses into fixtures/rules/golden/
snapshot-rules-wire port="5079":
    ./scripts/snapshot-rules-wire.sh {{port}}

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
# `--baseline-lenient`, not `--baseline`: criterion PANICS when a bench has no
# entry in the named baseline, so adding one new bench would abort the whole
# comparison partway. Lenient skips the missing one and compares the rest.
bench-compare name="baseline":
    LEDGELINE_BENCH_SIZES=5000,50000,200000 cargo bench -p ledgeline-core -- --baseline-lenient {{name}}

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
