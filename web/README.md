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
bun run check            # svelte-check + tsc --noEmit (just check)
bun run test:unit        # vitest — BOTH projects (just test)
bun run test:node        # vitest --project=unit        (pure functions, no DOM)
bun run test:components  # vitest --project=components  (mounted .svelte files)
bun run test:e2e         # playwright, browsers provided by nix (just e2e)
bun run lint             # prettier --check + eslint
bun run format           # prettier --write
```

### The two vitest projects

`vite.config.ts` declares two, split by what they need to run rather than by what
they test. Both are self-contained: no engine, no browser, no network.

|             | `unit`                    | `components`                     |
| ----------- | ------------------------- | -------------------------------- |
| Files       | `src/**/*.{test,spec}.ts` | `src/**/*.svelte.{test,spec}.ts` |
| Environment | `node`                    | `jsdom`                          |
| Speed       | ~0.5s for the whole suite | ~1s, and it mounts things        |

**Write a `unit` test by default.** A decision named and answered in a pure
function is tested by calling it with its inputs, which is cheaper to write,
faster to run and far easier to read than a test that has to build a screen to
reach the same branch. Everything under `lib/domain`, `lib/reports`,
`importModel.ts`, `model.ts` and the `nativeDecode` decoders is there so that
this is possible.

**Write a `components` test when the claim is about a mounted component**, which
in practice means one of two things:

1. **What a component was HANDED.** Every pure function behind the New
   Transactions screen was green while that screen opened with a spinner in an
   empty drop zone, a frozen destination form and a Save button spinning for a
   request nobody had made. The logic was right; the template asked the wrong
   questions of it. Only a mount can catch that.
2. **What happens on mount.** `AliasPanel` shipped an `$effect` that fed itself,
   threw `effect_update_depth_exceeded` and froze the entire app — every nav
   link and every button, with nothing visible on screen. There is no function
   to call that reproduces it.

Name the file `Thing.svelte.test.ts` beside its component and the include globs
route it to the right project automatically. `$lib/testing/` holds the shared
setup, the fake-engine `fetch` stub and the wire fixtures; component tests drive
the REAL stores through that stub rather than mocking them, because a mocked
store proves the component renders what it is given, and what it is given is the
thing that has broken twice.

Three things to know before writing one:

- **jsdom has no layout engine.** Nothing may assert on geometry, on
  visibility-by-overlap, or on computed CSS. Assert on structure, on accessible
  roles and names, and on the values a component was handed.
- **`render()` flushes, so `$effect` bodies have run by the time it returns** —
  no `await tick()` needed for a mount-time effect. But when the assertion is
  about an effect THROWING, wrap `render()` and an explicit `flushSync()` from
  `svelte` in the same expression: an effect that throws outside your closure
  surfaces as an unhandled rejection blamed on some later test.
- **Module-level runes state is shared by every test in a file.** The stores are
  singletons, so a test that stages a file has staged it for everything below it.
  Vitest gives each test FILE a fresh module registry, which is why the New
  Transactions screen is covered by two files (`…svelte.test.ts` for at-rest,
  `….staged.svelte.test.ts` for after a drop) rather than one.

Vitest's browser mode would be the more faithful renderer, and it is not used:
Chromium cannot launch in this environment (`bootstrap_check_in … Permission
denied`), so browser-mode tests would be runnable on CI alone — which is exactly
where a test stops being consulted while you work.

## Building

```sh
bun run build      # static SPA in build/ with index.html fallback (just build)
bun run preview    # serve the production build locally
```
