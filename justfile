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

# Production build (static SPA)
build:
    cd web && bun run build
