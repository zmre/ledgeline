# Ledgeline web

The SvelteKit SPA for Ledgeline. See `../plans/00-overview.md` for architecture and conventions.

All commands run through the Nix dev shell (`nix develop path:..` from this directory, or direnv at the repo root). Bun is the package manager — never npm.

## Developing

```sh
bun install
bun run dev        # or: just dev (from the repo root)
```

## Checks and tests

```sh
bun run check      # svelte-check + tsc --noEmit (just check)
bun run test:unit  # vitest (just test)
bun run test:e2e   # playwright, browsers provided by nix (just e2e)
bun run lint       # prettier --check + eslint
bun run format     # prettier --write
```

## Building

```sh
bun run build      # static SPA in build/ with index.html fallback (just build)
bun run preview    # serve the production build locally
```
