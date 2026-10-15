# TODO


## Known issues
- fix: display issue where pie chart is not round, but oval when the window narrows horizontally or vertically.  Update: seems to be specific to linux as I can't reproduce on mac. Static analysis says the geometry is fine — layerchart computes the radius as `min(width, height) / 2` and emits a true circle with no `viewBox`, and nothing in our CSS scales non-uniformly — so this needs a WebKitGTK repro (dump the `<svg>`'s rendered box, attributes and computed style from devtools on Linux) before anything is changed.

## Security
- security: `security::inline_script_hashes` decides "external" by `contains("src=")` on the open
  tag, so an inline `<script data-src=…>` goes unhashed (and is refused). SvelteKit emits no such
  tag today, and `tests/spa_shell_csp.rs` (whose scanner parses attribute names) fails if it ever
  does; still worth matching the attribute name in the production scanner.
- security: **some READS still follow a swapped-in symlink.** Writes are closed (`pinned`:
  resolve once, `O_NOFOLLOW`, `(dev, ino)` + fingerprint checked through the fd up to `renameat`),
  but `rules_api::read_document`, `projections_api::read_file` and every `hledger` invocation
  (`-f <journal>`) still open by path. A link swapped in mid-request can make a preview or a
  revision describe the link's target — a disclosure into the UI, never a write. Wants
  `pinned::read_pinned` at those reads and, for hledger, a check of the pin after each run.
- chore: **cap-sweep residue.** The import preview's `additions` (`import_api::cli_parity`) are
  still diffed against the config's *forwarded* aliases, capped at `MAX_CONF_ALIASES`, so past 200
  it can list a line the write then (correctly) does not add; the write path uses the full list.
  And "no ASCII control character" now has five private spellings (`accounts`, `periodic`,
  `aliases` `has_control`; `rules::check_text`; `edit_api::check_text`) — one shared helper.
- chore: `git.rs`'s `Repo::scrub` still swaps the repo toplevel without a path-boundary check
  (hiding `/u/fin` rewrites `/u/finances2/x`). The shared `redact::Redactor` fixed this everywhere
  else, but `tests/git_commit.rs` compiles `git.rs` standalone via `#[path]`, so it cannot use
  `crate::redact` as-is; lift the boundary check into a dependency-free helper both can share.

## Misc
- chore: Add screenshots and better descriptions to the readme
- ledgeline looks great, but it inherits the complexity and issues of web apps due to our architecture choices
  - these were the right choices for mbr (cuz markdown) but maybe not here
  - what if we used <https://iced.rs> or [GPUI](https://github.com/longbridge/gpui-component) or something? libcosmic? [freya?](https://github.com/marc2332/freya)
  - for forms and displays of numbers and such, it would probably be a great improvement
  - for charts, i expect we'd be in trouble; egui has some libraries that might do
- chore: publish docs to website using mbr

## Performance
- perf: **`/api/insights` misses its gate by 2.4×** — 968 ms at 200k against a 400 ms target, and it
  is the dashboard's landing view, so it is the first number anyone feels. Insights' own dedup work
  is correct; the cost moved. `compute_holdings` regressed **+57% at 200k / +96% at 50k** under
  feature accretion (Other-holdings tab, subtree rollups, `valuation:`, cost-basis price-route
  warnings — each added a pass), and insights calls it 3 times / ~5 engine passes: twice via
  `window_holdings`, each self-recursing because `gain_since` is set, plus once through
  `movers` → `portfolio_at`. Holdings work is roughly two-thirds of the 968 ms. Memoizing one
  holdings replay across the three calls should get insights to ~400-500 ms.
- perf: **`value_at` rebuilds the entire `PriceGraph` on every call that misses a direct price.** The
  graph is a function-local, built and dropped per call, and `graph_at` is O(P). Invisible today
  because a `$`-only book always hits the direct-lookup fast path — the code's own comment records
  the cliff as ~6 ms vs ~215 ms. Exposure is per call site: `reports/flows.rs:657` sits inside
  `for txn in txns` × accounts-in-txn, so **up to 150k graph builds per request**. Also
  `net_worth.rs:253` (A × D builds) and `holdings/other.rs:350` (per bucket). Fix is to hoist the
  graph to a caller-owned parameter or memoize it on `PriceDb` keyed by `as_of` — a signature change
  across ~8 call sites, hence not a quick one. This is the difference between "fine" and "unusable"
  on any journal that is not single-currency.
- perf: **no windowing on `/transactions`.** ETag and compression are in place (so an unchanged poll
  is free) but the first load is still all-or-nothing — no offset/limit/date-range param exists on
  either end. ~347 MB at 200k. This is the structural blocker for a large journal in the UI and it
  needs both the route and the SPA. It is also what keeps ~390 MB of SPA peak memory in place, since
  the raw wire payload, the normalized journal and the previous normalized journal are all held at
  once.
- perf: a single `/api/insights` request performs **7 `PriceDb::build`s**, each deep-cloning every
  directive — ~500k clones per request on the v2 corpus.
- perf: `choose_base` → `coverage` is where the entire cost of a mesh price graph lands (+39% on
  `compute_holdings`, +45%/+56% on the holdings series at 200 commodities, and none of it in the
  reports). It is guarded by "fewer than 2 candidates", so it is free on a `$`-only book and a cliff
  the moment there is a cross-rate plus any single unpriceable symbol.
- perf: `reachable` (`reports/prices.rs:274`) re-scans the whole edge slice per popped node, so it is
  O(V·E), **not the O(V+E) its comment at `prices.rs:272` claims**. `shortest_path` enumerates simple
  paths with a per-partial `Vec` clone and `PriceGraph::rate` runs it twice. Fine on a `$`-star price
  graph, super-polynomial on a mesh.
- perf: **`latest_directive_price` residual.** The reverse walk exits early only on a base-priced
  directive, so a symbol that is *never* priced in the base scans its whole prefix — the v2fx
  corpus's 59 such symbols cost 0.60 ms per series-point against v2's 0.20 ms. The index could answer
  "this symbol has no base-priced directive at all" and let it exit immediately.
- test: **a third corpus shape with a genuinely DENSE many-to-many FX graph.** This is what the
  `value_at` / `PriceGraph` finding above actually needs — `v2fx` is a star-of-stars with 2-3 hop
  chains and low degree, and never exercises `shortest_path`, so that finding is still unmeasured
  rather than disproved.
- test: `examples/load_rss` takes a transaction count rather than a `Shape`, so there is **no
  peak-RSS number for v2 or v2fx** — the memory dimension of the 200-commodity shape is unmeasured.

### Never examined — so "not on the findings list" does not mean "measured and fine"
- **XLSX export.** The balance-sheet / income-statement / holdings exports run through `exceljs` in
  the browser and have never been profiled. A large report on a big journal is the obvious risk.
- **Real app startup**, window-open to first paint. `snapshot_from_journal` is measured (458 ms at
  200k) but the end-to-end desktop launch — wry/WebKit init, SPA boot, first fetch — is not.
- **The import/convert path at scale.** `MAX_INPUT_BYTES` is 16 MiB; the converter and the rules
  matcher have never been benched against a file near that, and `import` shells out to hledger
  several times per dry-run with a 120 s timeout each.
- **Concurrency.** Report work runs on `spawn_blocking` behind a core-count semaphore, but there is
  no HTTP-level benchmark, so behaviour under several simultaneous report requests is untested.

### Front end

The default filter is last-90-days, which is what hides journal-size problems; the cliff is the
user's first click on "All time".

- fix(web): **a slow insights refetch shows stale numbers under the new period control, silently.**
  `createResource`'s `view` is `dataView(status, loaded !== null)` with `matchesRequest` defaulting
  to true, and `dataView` returns `"data"` when `status === "loading"` and a payload exists (pinned
  at `loadState.test.ts:17`). Keeping the old data instead of blanking is the right call for
  perceived speed — but `InsightsDashboard` has **no pending indicator of any kind** (no dimming, no
  spinner, `PeriodControl` is not disabled), so for the length of the request the boxes say "Last 3
  months" while the control says "Last 12 months" and nothing signals it. Wants a subtle refreshing
  state, not a blank.
- perf(web): the whole insights dashboard is **one `AsyncSection`** (`InsightsDashboard.svelte`), so
  the first load is all-or-nothing: one centred spinner, then every box at once. The endpoint
  computes the boxes from shared intermediates, so progressive population would mean splitting
  `/api/insights` — worth it only if the endpoint stays slow.
- perf(web): **virtualize or cap the Problems drawer list.** The `{#if}` guard moved the cost off
  every page but did not remove it: building 21,429 findings now happens when the drawer *opens*,
  measured at 4.3 s in jsdom — much faster in a real browser, but still a visible pause. Virtualize
  the list the way `TransactionTable` already virtualizes rows.

## Import improvements

- chore: `AmountStyle.digit_groups` is cloned per amount. The payload is bounded
  (`MAX_DIGIT_GROUPS`) so there is no amplification, but the real fix is
  `digit_groups: Option<Arc<DigitGroups>>` in `model.rs` — deferred because it is a cross-cutting
  change to a widely-used type. There is a `TODO:` at the field.
- Can we setup auto-syncing using SimpleFIN / SimpleFIN Bridge?
  - Is that still maintained?  Does it still work?
  - Do we have any other reasonable sync approaches?
  - Can we run it ourselves?

## AI

- feat: private AI integration
  - Need to make use of a per-user preference specifying the url for the AI and any necessary api keys or whatever
  - Need to make a way for the user to edit this in the app
  - Only show an AI icon if we have a successful connection; or maybe there's an AI chat icon that has a red dot over it if the configured url doesn't work, a green dot if it is working, and the icon is hidden if this isn't setup (under settings tab)
  - clicking the chat slides over a drawer
  - we need to build out a system prompt with information about the files in the folder. i think we need to allow a tool call to fetch files in the folder, but nothing else. and a tool call that could write to files, but with strict user approval checks.
  - i have mostly used ai functionality on my hledger repos when i want to use an external file -- often a pdf -- and produce some specific journal entries from it such as balance checks or setting up a partnership or doing fair value / nav adjustments
  - ultimately, the AI should interact with the user through the chat drawer. the user should allow it to see specific files or all files as they desire. user should be able to clear history.  ai should be able to request reading of files and propose changes to files.
  - i don't want to reinvent too much here.  and i'd like to use our API endpoints rather than any direct writing. maybe that's what we offer as tools is our api endpoints, but with per-session user approval?  i mean, if the AI is private, it's probably fine to offer up the read-only endpoints. the main thing is to gate writes, not reads.

## Stocks

- feat: benchmark comparison lines need a USD valuation base. Convert the dividend-adjusted closes through journal prices so a non-USD book gets them too.

## Ledger 

- feat: intelligent category suggestions
  - only real way to do this is with some sort of lookback comparing similar descriptions in the past and seeing associated expense or revenue accounts
  - need to remove random numbers from description and maybe do a predominance calculation or a vector comparison rather than full equality.  if we're doing equality and removing numbers, we need to normalize some by lowercasing.  but in a perfect world, "netflix.com" might see a previous "netflix" and guess category based on that.  the more exact the match and the more recent, the higher the sort ranking
  - feat: remember categorization functionality — write a chosen category back into the rules file as a new `if` rule (the rules editor and its write path are done; this is the one-click path into them)
- feat: File -> New
  - Here I'm assuming we're setting up a new set of journal files, chart of accounts, etc. Probably we prompt with some questions and use an empty folder as a starting point and then create a skeleton so someone can start using us to track things. We should have a default chart of accounts for individuals and another for businesses and then we should allow them to start with an empty set of accounts to add their own.
- feat: add xlsx download to the journals tab
  - it should respect account filtering (including search) and date filtering.
  - the idea here is this is what you would send as the journal/ledger info to an accountant who needed details while doing taxes or something
  - the challenge here is when a single entry has more than two accounts, so we'd probably do this as multi-line. see if you can research how quickbooks exports journal/ledger reports, which i think we already know because we can import them. if not, i can get a sample.  but lets try to be compatible with that.
- feat: saved report filters?

## Budgeting / planning / modeling

- feat: personal planning calculators a la quicken financial planner; see inspiration from [credit karma](https://www.creditkarma.com/calculators/money) and [nerdwallet](https://www.nerdwallet.com/investing/calculators)
  - great free tools with details at [engaging-data](https://engaging-data.com/early-retirement-calculators-and-tools/)
  - TODO: investigate [projection lab](https://projectionlab.com) to understand if that's worthwhile or anything there we want to learn from. from a friend: "really nice stuff built on top of it (roth conversions, drawdown simulation, flex spending, tax strategy, "what if" checkpointing to compare decisions, nice milestone tools to setup when costs are known to change and how, etc"
