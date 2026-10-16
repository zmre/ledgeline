# Docs screenshots

Playwright specs that regenerate the PNGs under `docs/**/images/`. They are not
tests and are not part of `just e2e`.

## Run

```sh
just docs-screenshots                      # build SPA + engine, then every shot
cd web && bunx playwright test -c playwright.docs.config.ts journal   # one spec, after a build
```

Run inside the dev shell (`nix develop`), which provides the browsers. Then look
at the changed PNGs and commit them with the UI change that moved them.

## How it works

- `playwright.docs.config.ts` starts one engine per fixture set (`sample`,
  `budget`, `subscriptions`) on ports 5211–5213. Each engine serves the SPA it
  embeds, same-origin, so `just docs-screenshots` builds the SPA before the
  binary.
- `stage.ts` copies each set into `target/docs-screenshots/<set>/` first. The
  engines never open `fixtures/` directly, so a shot cannot change a committed
  fixture. The `sample` copy also gets an `imports/` folder with two rules files
  and three `alias` lines that no posting uses.
- `harness.ts` pins the clock to 2026-07-08 and the timezone to America/Denver,
  and fills the same date into the two Projections requests that otherwise use
  the engine's own clock. It also provides:
    - `annotate(page, specs)`: draws `circle`, `box` and `arrow` (with an
      optional `label` pill) marks from locator boxes in a fixed SVG overlay, and
      returns the drawn bounds. `clear(page)` removes them.
    - `shot(page, section, name, {clip, padding})`: waits for spinners to go,
      freezes animations, hides the caret and writes
      `docs/<section>/images/<name>.png`. Budget shots go to
      `docs/images/budget-<name>.png`. `clip` takes locators and rects; pass the
      `annotate` result too so the marks stay in frame.
    - `around(targets)`: one rect covering several controls.
- Viewport 1280×800 at 2× scale. The app has a single (dark) theme.

## Add a shot

1. Pick the spec for the manual section (`journal.docs.ts`, `imports.docs.ts`, …).
   For a different journal use `test.use({journal: "budget"})`. For a new
   fixture set, add it to `stage.ts` and `DOCS_ENGINES`.
2. Navigate, wait on something that only appears once the data has loaded, and
   then call `annotate`/`shot`. Prefer excerpts (`clip`) over full screens when
   the page text is about one control.
3. Run the spec, open the PNG, fix the crop, and add a row to `MANIFEST.md`.

Nothing in a spec should press Save, Import, Map accounts or Update prices.
