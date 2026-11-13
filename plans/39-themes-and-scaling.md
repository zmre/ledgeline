# Plan 39 — themes (with OS light/dark) and interface scaling

Branch `2026-11-12-visual-enhancements`, built on PR #62 (`2026-11-11-fixes`,
plan 38 WP-5: `ui-prefs.toml` + `ledgeline.toml` overrides), merged in first.

## The ask (Patrick, 2026-11-12)

- Themes, remembered per ledger. Respect the OS light/dark setting; today the
  app is dark only.
- Start from popular developer themes. A saved theme is used as-is (One Dark
  Pro stays dark in light mode). The default theme, `default`, follows the OS.
  Families that have both variants (GitHub, Catppuccin, …) can follow the OS
  too.
- Interface scaling: a setting plus Cmd/Ctrl `+`, `-`, `0`. Every text size
  derives from one base; test the limits and fix layouts that break.

## Decisions

### Theme ids — one key, `theme`

A **family id** follows the OS (`prefers-color-scheme`) when the family has
both variants; a **variant id** pins one. Dark-only families have one id.

| Family        | Follows OS id | Light variant       | Dark variant         |
|---------------|---------------|---------------------|----------------------|
| Default       | `default`     | `default-light`     | `default-dark`       |
| GitHub        | `github`      | `github-light`      | `github-dark`        |
| Catppuccin    | `catppuccin`  | `catppuccin-latte`  | `catppuccin-mocha`   |
| Gruvbox       | `gruvbox`     | `gruvbox-light`     | `gruvbox-dark`       |
| Tokyo Night   | `tokyo-night` | `tokyo-night-day`   | `tokyo-night-night`  |
| Solarized     | `solarized`   | `solarized-light`   | `solarized-dark`     |
| One Dark Pro  | —             | —                   | `one-dark-pro`       |
| Dracula       | —             | —                   | `dracula`            |
| Monokai       | —             | —                   | `monokai`            |
| Nord          | —             | —                   | `nord`               |

- The `data-theme` attribute on `<html>` is always a **variant** id; the
  family → variant resolution is a pure, tested function of `(theme pref,
  prefersDark)`.
- `default-dark` is today's daisyUI `dark` (same chart palette, unchanged).
  `default-light` is daisyUI `light` plus a new chart palette.
- Every variant is a daisyUI custom theme (`@plugin "daisyui/theme"`) built
  from the upstream theme's published palette, with `color-scheme` set, and its
  own chart block (`--chart-1…N`, `--chart-other/in/out/net`) validated with the
  dataviz skill's validator against that variant's `--color-base-100`
  (adjacent CVD ΔE, normal ΔE, ≥ 3:1 contrast). Results recorded in a comment
  per block like the existing dark one.
- Unknown `theme` value → `default` (validated server-side like any Choice).

### Scale — one key, `uiScale`

- Whole percent, steps `70 80 90 100 110 125 150 175 200`, default `100`.
  Server validates the range 50–300 (hand edits) and clamps; the UI snaps to
  steps.
- Applied as `html { font-size: calc(var(--ui-scale) * 100%) }`, so all `rem`
  (Tailwind, daisyUI) follows. Arbitrary px text (`text-[11px]`) becomes rem.
- JS px constants that encode rendered text or row geometry (row pitch, card
  height, chip font for `textWidth`, axis/net-label px, SVG `fontSize`, popup
  `ROW_HEIGHT`s) are expressed in rem and converted with the live root px.
  Measure caches key on the effective px.
- Shortcuts: Cmd (mac) / Ctrl (else) `=`/`+` in, `-` out, `0` reset, through
  the keymap with `preventDefault`. Desktop: a View → Zoom In / Zoom Out /
  Actual Size menu with those accelerators, which dispatches to the SPA
  (`evaluate_script`); the SPA owns the step logic and the save.

### Per ledger — explicit pin

- Default routing is unchanged (plan 38): saves go to `ui-prefs.toml` unless
  the ledger's `ledgeline.toml` already sets the key.
- New: the theme picker has **"Use for this ledger only"**. Ticking it pins
  `theme` in `ledgeline.toml`'s `[ui]`, **creating the file if needed** — the
  one user-initiated exception to "the app never creates `ledgeline.toml`".
  Unticking removes the key from `[ui]` (never deletes the file). While pinned,
  theme changes save locally — that is the existing routing.
- Pinnable keys are an allowlist: `theme`, `uiScale`. Opening the app still
  writes nothing.

### First paint

The prefs arrive by fetch, so the boot uses a localStorage cache of the last
resolved variant and scale (a cache, never the source of truth) and otherwise
`default` + the OS scheme. The desktop origin changes per launch, so it may
flash once; fixing that by templating `index.html` is a follow-up.

## Work packages

- **WP-A prefs (Rust + wire)** — `theme` Choice and `uiScale` whole specs;
  pin/unpin in `PATCH /api/ui-prefs` with create-on-pin through the vault;
  TS types, store setters; endpoint + unit tests; docs/settings.
- **WP-B themes** — the 16 variants in CSS, chart palettes validated, palette
  tests generalized to every theme block, a theme catalog module
  (ids, labels, family, scheme, swatch colours) and the resolver.
- **WP-C scale + shortcuts** — root scale variable, px → rem conversions,
  JS geometry from root px, keymap bindings, desktop View menu.
- **WP-D integration (after merge)** — apply theme/scale from prefs reactively
  (incl. `matchMedia` changes), boot cache, Settings → Appearance tab (theme
  grid with swatches, OS-follow note, pin checkbox, scale stepper), charts
  re-resolve on theme change, e2e for theme + zoom, screenshots at 70 % and
  200 % across every route, fix what breaks.

## Status (2026-11-12)

WP-A–D done; the 70 %/200 % screenshot pass found the header and breakpoints
broken at large scales. Fixed by redefining Tailwind's `sm`–`2xl` as container
queries on `<body>` (rem there tracks the root font size; in media queries it
doesn't) and folding the section tabs into a menu below `md`.

### Follow-ups

- ~~Debounced save lost on quit~~ — done: theme and text size save at once
  (serialized so the last value wins), and other pending saves go out with a
  `keepalive` PATCH on `pagehide` / hidden.
- ~~Desktop first-paint flash~~ — done: the engine resolves the saved theme
  (`ui_prefs::boot_appearance`) and the webview's init script seeds the boot
  cache once per webview session.
- Not yet verified in the desktop app (WebKit): one keypress zooms exactly
  once via the View menu; `container-type` on `<body>` behaves as in Chromium.
- Imperfect at the limits: Find Matches tooltip widens the journal 47 px at
  800×600/200 %; chart axis text (0.625rem) and 0.65rem report labels are
  < 8 px at 70 %; some axis labels touch at 200 %; wide tables scroll.
- `max-*:` and range breakpoint variants are still media queries (unused today).
