> **Superseded:** describes the original read-only SPA build; the app is now a Rust single-binary engine + GUI — see `docs/development.md` and the repo README.

# WP-01: SvelteKit Scaffold

Read `plans/00-overview.md` first.

## Scope

Create the SvelteKit app in `web/` with the full toolchain configured: static SPA, Tailwind v4 + daisyUI v5 dark theme, LayerChart, prettier/eslint, vitest, playwright, app shell layout with navigation and two empty routes. This WP is the **gate** for all others — everything must build, check, and run.

## Out of scope

Any data fetching, stores, domain logic, real components beyond the shell.

## Steps

1. From repo root, inside the dev shell: `cd web` (create dir) and scaffold with `bunx sv create .` — options: TypeScript (strict), add-ons: tailwindcss, vitest, playwright, prettier, eslint. Use Bun as package manager throughout (`bun install`, `bun run ...`).
2. Static SPA config:
   - `bun add -d @sveltejs/adapter-static`
   - `svelte.config.js`: `adapter-static({ fallback: 'index.html' })`
   - `src/routes/+layout.ts`: `export const ssr = false; export const prerender = false;`
   (Pure SPA: the API URL is runtime state from localStorage; nothing can be server-rendered.)
3. Tailwind v4 + daisyUI v5: `bun add -d daisyui`, then in `src/app.css`:
   ```css
   @import "tailwindcss";
   @plugin "daisyui" {
     themes: dark --default;
   }
   ```
   `<html data-theme="dark">` in `src/app.html`.
4. Charts: `bun add layerchart` (v2+, Svelte 5 compatible). Verify it compiles by importing one component in a scratch page, then remove.
5. Prettier per house style (`.prettierrc`): `{"printWidth": 160, "tabWidth": 4, "trailingComma": "es5", "bracketSpacing": false, "arrowParens": "always"}` + svelte/tailwind plugins from the scaffold.
6. tsconfig: extends `.svelte-kit/tsconfig.json`; ensure `strict: true`, add `noImplicitReturns`, `noUnusedLocals`, `noUnusedParameters`.
7. App shell:
   - `src/routes/+layout.svelte`: top navbar (daisyUI `navbar`) with app name and two links — Journal (`/`) and Reports (`/reports`); slot for a connection-status indicator (WP-02 fills it); responsive (navbar collapses cleanly at mobile widths).
   - `src/routes/+page.svelte`: placeholder "Journal" heading.
   - `src/routes/reports/+page.svelte`: placeholder "Reports" heading.
8. Package scripts (ensure these exist; justfile calls them): `dev`, `build`, `preview`, `check` (svelte-check + tsc), `test:unit` (vitest), `test:e2e` (playwright), `lint`, `format`.

## Key files created

`web/package.json`, `web/svelte.config.js`, `web/vite.config.ts`, `web/tsconfig.json`, `web/.prettierrc`, `web/src/app.html`, `web/src/app.css`, `web/src/routes/{+layout.svelte,+layout.ts,+page.svelte}`, `web/src/routes/reports/+page.svelte`

## Depends on / parallel

Depends on: nothing (repo root files exist). Parallel with: nothing — all other WPs wait on this.

## Definition of done

- `just build` produces a static site in `web/build` with `index.html` fallback
- `just check` zero errors; `just dev` serves the shell; nav works between `/` and `/reports`
- Dark daisyUI theme visibly applied; shell usable at 375px and desktop
- `bun run test:unit` and playwright scaffold run (even with only placeholder tests)
- Commit: `feat: scaffold sveltekit spa with tailwind/daisyui dark theme`
