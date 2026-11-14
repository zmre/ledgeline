# Plan 42: iOS / iPadOS app

Status: Phases 1–2 in progress; phase 3 (gestures + lock) done (2026-11-13). Not for the App Store yet: a
development-signed build installed straight onto Patrick's devices.

## Goal

A native iOS/iPadOS shell around the existing SPA, built the way MBR's
mobile app is (`mbr-markdown-browser/apple/ios`):

- **Home screen** (SwiftUI) lists saved ledgers; add more through the
  document picker; swipe/context menu to rename or remove. Tapping one opens it.
- **Native navigation bar** replaces the SPA's HTML header (hidden in
  native mode). Title = current section (subtitle = ledger name).
- **Top-left back**: back within the SPA (closes an open overlay first, then
  history); at the first screen it returns to the home screen.
- **Top-right hamburger** holds everything the header used to: the sections,
  the current page's actions (export, refresh, problems, …), then
  native items (Switch Ledger, Settings).
- **Screen size decides layout, not platform.** An iPad gets the desktop
  layout where it fits; narrow layouts key off the existing container-query
  breakpoints (`sm`…`2xl`, plan 39), never off "is iOS".

Groundwork already done in plan 26: `crates/ledgeline-ffi` (`mobile`
feature, UniFFI `LedgelineApp`, `Router::oneshot` transport, `FileCoordinator`,
dataless-file handling) and `scripts/ios-check.sh` (cross-compile shims).

## Architecture (copied from MBR)

| Piece            | Choice                                                                                       |
|------------------|----------------------------------------------------------------------------------------------|
| Project          | XcodeGen `apple/ios/project.yml`; `.xcodeproj`, xcframework, build dirs git-ignored         |
| Rust             | `ledgeline-ffi --features mobile`, staticlib → `LedgelineFFI.xcframework` (device + sim)    |
| Bindings         | `uniffi-bindgen generate --library … --language swift` → `apple/ios/Generated/` (checked in) |
| Web              | SwiftUI `WebView`/`WebPage` (iOS 26), `URLSchemeHandler` for `ledgeline://localhost/`       |
| SPA assets       | embedded in the staticlib (rust-embed); **release** profile only (debug reads `spa/` from disk) |
| Min iOS          | 26.0; `TARGETED_DEVICE_FAMILY` 1,2; bundle id `com.zmre.ledgeline.ios`; team `X2SAY6ACE6`  |
| Build driver     | `apple/ios/build.sh {rust,build,run,device,install}` + `just ios-*` recipes                  |
| Saved ledgers    | `UserDefaults` JSON list of `{id, name, mainFile, bookmark}`; caches in `Caches/ledgers/<id>` |

## The native bridge contract (JS ↔ Swift)

Both sides MUST match this; a vitest reads the Swift file and checks every
literal (as MBR's `app-bridge.test.ts` does).

**Swift → page at document start** (`WKUserScript`, main frame only, after
`tokenInitScript()`):

```js
window.__LEDGELINE_NATIVE__ = {platform: "ios", idiom: "phone" | "pad", version: 1};
document.documentElement.classList.add("ll-native");
```

**Page → Swift**: `window.webkit.messageHandlers.ledgeline.postMessage(msg)`

| `type`  | Payload                                                                                                         | When                                         |
|---------|-----------------------------------------------------------------------------------------------------------------|----------------------------------------------|
| `page`  | `{title, sections:[{id,label,active}], actions:[{id,label,icon?,destructive?}], canGoBack}`                     | after every navigation / state change (deduped) |
| `share` | `{filename, mime, base64}`                                                                                      | xlsx (or other) export instead of `<a download>` |
| `haptic`| `{style: "light"\|"success"\|"warning"}`                                                                         | optional feedback                             |

- `sections[].id` ∈ `journal holdings budget projections reports settings`
  (Imports is hidden in native mode).
- `actions[].icon` is an SF Symbol name; Swift falls back to none.
- `canGoBack` = an overlay/dialog is open OR the SPA has in-app history
  beyond the first page.

**Swift → page** (`callJavaScript`):

```js
window.ledgelineNative.navigate(sectionId)   // -> bool
window.ledgelineNative.action(actionId)      // -> bool
window.ledgelineNative.back()                // -> bool; false = nothing to go back to (Swift shows home)
window.ledgelineNative.state()               // -> the last `page` payload
```

## What changes in native mode / at narrow widths

Native mode = `html.ll-native` (set only by the iOS shell). Narrow = below
the `md` container breakpoint (48 rem).

| Area                      | Change                                                                                       | Keyed on       |
|---------------------------|----------------------------------------------------------------------------------------------|----------------|
| HTML header               | collapsed (height 0, not `display:none`, overlays live there); safe-area top padding        | native         |
| Section nav, settings gear, refresh, problems badge, status dot | moved to the hamburger as sections/actions                 | native         |
| Imports, New-ledger wizard| hidden (needs hledger subprocess / desktop host)                                            | native         |
| Keyboard hints (`kbd`, "(v)" in titles) | hidden                                                              | no fine pointer (`any-pointer: fine` false) |
| Calendar view             | toggle removed; `cal=1` opens the list                                                      | narrow         |
| xlsx export               | `share` message → native share sheet (Save to Files, AirDrop, Mail…)                         | native         |
| Reports / projections tables | horizontal scroll with sticky first column                                               | narrow         |
| Journal list              | three-line cards (date · payee · amount, then the full accounts wrapping onto two lines, tags, status) instead of columns; a tap (status badge included) opens a read-only detail sheet (every posting, tags, comments as free text without their tags) with only Edit and Mark reviewed; a tap outside or back closes it | narrow |
| Journal scroll            | one page scroll: the windowed list follows the page (`pageScroll`), no inner scroller; the totals footer flows after it | narrow |
| Journal review buttons    | one Review menu button with the total as its badge: Review unknowns (n), Review recents (n), Find matches (n), then both settings | narrow |
| Journal cards             | swipe right = mark reviewed (unreviewed stripe) else edit; swipe left = Edit / Delete (Delete arms the floating confirm); long press = Edit | narrow, touch |
| Control bars              | Budget, Reports, Projections and the Holdings scope bar fold behind a one/two-line summary (`FoldedControls`) that opens them | below `sm` |
| Sample ledger             | always the pristine sample, every date moved by whole months so the newest transaction is in the current month (`date_shift.rs`; desktop "Try with sample data" and the iOS sample, via `sample_journal()` over FFI, rewritten on every open) | all |
| Sideways scrollers        | `overscroll-behavior-x: contain` on every horizontal scroller; the web view bounces sideways only when the content is wider (`scrollBounceBehavior(.basedOnSize, axes: .horizontal)`) | all / native |
| Bottom inset              | `--ll-bottom-inset`: the app measures the home indicator and sets `--ll-native-bottom` (the web view ignores the bottom safe area) | native |
| Page zoom                 | WebKit zoom off (viewport `user-scalable=no`, set by the app's script); pinch steps the UI scale via `ledgeline:zoom` | native |
| Entry dialog, review overlay | full-screen sheet layout, stacked fields                                                 | narrow         |

## Phases

### Phase 1: shell + native chrome (now)

- iOS project, build script, xcframework, Swift app: home screen with
  multiple ledgers, `WebPage` + scheme handler + coordinator, native bar,
  back, hamburger, share sheet, refresh on foreground and pull-to-refresh,
  privacy cover in the app switcher.
- SPA: `lib/native/` bridge implementing the contract above; header hiding;
  Imports/New hidden; kbd hints hidden without a fine pointer; xlsx via share;
  calendar removed at narrow widths.
- FFI: give mobile a config dir (Application Support) so `ui-prefs.toml`
  persists and the "no config folder" toast never shows.
- Install on Patrick's iPhone 15 Pro.

### Phase 2: screen-by-screen narrow layouts

Audit every screen at 390 pt (iPhone), 744 pt (iPad mini portrait) and
1024 pt (iPad landscape) in the simulator; then: journal cards, filter bar,
entry dialog, review/match overlays, reports & projections horizontal scroll,
budget, holdings, settings panels.

Done so far (2026-11-13): journal cards, review labels above fields, sticky
report label columns; tab strips (Reports, Projections, Settings) scroll on one
line and reveal the selected tab (`HScroll reveal`); budget controls fold
behind a summary line below `sm`; holdings tables get a sticky name column with
the symbol/account under the name below `md`; projection cards give postings
the full card width below `sm`; the forecast journal drops its date column
below `sm`. Settings → Prices is hidden in the app (no background updater
runs there).

Zoom pass (2026-11-13, iPhone feedback): every screen (journal, review and
edit cards, detail sheet, reports, holdings, budget, projections, every
Settings tab) swept at 100/150/200 % for boxes past the viewport and fixed;
iPad 125/175 % checked as the narrow-desktop case. Journal cards are three
lines (full accounts) and a tap opens a read-only detail sheet; one Review
menu replaces the three narrow launchers; review/match cards scroll only
vertically; dropdown panels stay on screen when their row wraps; sideways
scrollers contain their overscroll and the web view no longer bounces
sideways. Matching from the sample ledger has no proposals, so the match
card at 200 % is unverified in the simulator.

Third iPhone round (2026-11-13): the detail sheet keeps only Edit and Mark
reviewed (tap outside or back closes it) and shows comments without their
tags; long press on a card = Edit; the card's status badge opens the sheet
instead of cycling; the review card's "Similar unreviewed" heading stays on
one line with its hint below; Reports, Projections and Holdings fold their
controls like Budget (shared `components/FoldedControls.svelte`); holdings
stat tiles are 2×2 below `sm` with values that shrink to fit; the sample
ledger (desktop and iOS) is shifted by whole months so it always ends this
month.

### Phase 3: native gestures + lock — done (2026-11-13)

Done: swipe/long-press on cards (`journal/SwipeCard.svelte`, thresholds in
`swipe.ts`, mark-reviewed in `review/markReviewed.ts`); left-edge swipe =
the bar's back (`EdgeSwipe` in LedgerScreen.swift, light haptic); pinch =
one UI-scale step per 1.25× (`PinchSteps.swift`, saved like desktop zoom);
`AppLock.swift` asks on open and after > 5 min in the background (cover
stays up with Unlock / Ledgers), per-ledger "Require Face ID" in the
ledger row's context menu and leading swipe (turning it off authenticates
first). Gestures were exercised in the simulator with synthetic pointer
events only; a real finger on the device is still to be tried.

Original scope:

- Swipe actions on journal cards (e.g. leading = review/edit, trailing =
  delete/duplicate), long-press context menu.
- Left-edge swipe = back (matches the native back button).
- Pinch on reports/journal = UI scale (`ledgeline:zoom` event already exists).
- Face ID / Touch ID / passcode (`LAContext.deviceOwnerAuthentication`) when
  opening a ledger and when returning after N minutes in the background;
  per-ledger toggle, on by default.

## Known gaps / deferred

- Spreadsheet imports spawn a worker process (`workbook_sandbox.rs`), not
  possible on iOS; moot while Imports is hidden.
- CSV-rules imports need hledger: Imports stays hidden.
- No new-ledger creation on device yet (pick an existing journal).
- **Single-file providers (Sync.com, Google Drive, OneDrive) are out of
  scope** (decided 2026-11-13). Their pickers grant files, not folders, so a
  journal with `include`s can't open and new files (scenarios, includes)
  can't be created. A "granted-files vault" (multi-pick each include, one
  bookmark per file) is possible but was judged not worth it while features
  would still be missing. Supported: iCloud Drive, On My iPhone/iPad, and
  other providers that grant whole folders (e.g. Working Copy).
