# Ledgeline task runner. Run `just --list` for an overview.

# Start the SvelteKit dev server
dev:
    cd web && bun run dev

# Serve the fixture journal over the hledger-web JSON API (read-only, CORS open)
serve-api:
    hledger-web -f fixtures/sample.journal --serve-api --cors='*' --allow=view

# Serve a real journal over the JSON API: `just serve-journal ~/finance/2026.journal`
serve-journal file:
    hledger-web -f {{file}} --serve-api --cors='*' --allow=view

# Run unit tests (vitest)
test:
    cd web && bun run test:unit

# Run e2e tests (playwright)
e2e:
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
