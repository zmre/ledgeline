# TODO


## Known issues
- fix: display issue where pie chart is not round, but oval when the window narrows horizontally or vertically.  Update: seems to be specific to linux as I can't reproduce on mac. Static analysis says the geometry is fine — layerchart computes the radius as `min(width, height) / 2` and emits a true circle with no `viewBox`, and nothing in our CSS scales non-uniformly — so this needs a WebKitGTK repro (dump the `<svg>`'s rendered box, attributes and computed style from devtools on Linux) before anything is changed.

- fix: the desktop app forgets every browser-stored setting on each launch. The GUI binds an ephemeral port (`crates/ledgeline-server/src/gui.rs`), and WebKit keys localStorage by origin, port included, so each launch starts with empty storage. ⌘R and File→Open keep the port, so the settings survive those. Lost on relaunch: `lastProjectionId`, column choices, chart toggles, benchmarks, review limits. `~/Library/WebKit/com.zmre.ledgeline/WebsiteData/Default/` holds one folder per past port. Fix by giving the webview a stable origin (custom scheme or fixed data-store key) or by moving these settings server-side. Found while writing plan 33.

## Import improvements

- **Syncing:** Can we setup auto-syncing using SimpleFIN / SimpleFIN Bridge? Is that still maintained?  Does it still work? Do we have any other reasonable sync approaches? Can we run it ourselves?

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

- feat: **Reconcile mode** (builds on plan 36's running balances). Pick an account, enter a statement's end date and closing balance, then tick off transactions as they match the statement, writing hledger's `*` cleared status on the account's posting. Show cleared total vs statement balance and the difference live, so you're done when it reads zero. Then offer to write a balance assertion at the statement date. Questions: posting-level vs transaction-level status (hledger has both; the posting is the right one here), `!` pending for items in flight, and whether "cleared through" gets remembered per account (a tag on the account declaration?) so the next statement starts where the last ended.
* Support editing assertions: add a "+ assertion" control to each posting row, backed by a new SetPostingAssertion op. The engine already writes assertions out (render_assertion), so most of the work is the new op, the UI and tests.
- feat: File -> New
  - Here I'm assuming we're setting up a new set of journal files, chart of accounts, etc. Probably we prompt with some questions and use an empty folder as a starting point and then create a skeleton so someone can start using us to track things. We should have a default chart of accounts for individuals and another for businesses and then we should allow them to start with an empty set of accounts to add their own.

## Search

- Our search should line up better with hledger search so you can copy an hledger query in and get the same results
  - This especially means smarting handling of tags when searching
- feat: saved report filters (needs to remember dates as absolute or relative as part of this, not just the search filter)

## Startup metrics (follow-ups to plan 37)

- **Counting commodities (tier D):** headcount (`(kpi:headcount) 1 FTE`), seats, units; revenue per FTE. Find them by a commodity tag, not by name.
- **Inventory metrics:** `kpi: inventory`, inventory turns, DIO, GMROI, sell-through, built on Holdings' lot/cost logic.
- **Churn grace period:** treat a customer as churned only after N consecutive zero months (today N = 1, so a credit month shows as churn + reactivation).
- **Spread-invoices fallback:** for books that can't tag recognition lines, spread each `customer:`-tagged invoice over 12 months (or a `term:` tag) and reconcile against recognised MRR, showing coverage.
- **Magic number** and a **cohort retention chart**.
- **Default projection:** extrapolate `kpi: recurring` revenue as run-rate and stop repeating non-recurring revenue.
- **AR aging** (0–30/31–60/61–90/90+), on top of invoice ↔ payment matching.
- **Docs screenshot** of the startup section, if the docs-screenshot harness can serve `fixtures/saas/startup.journal`.

## Budgeting / planning / modeling

- **Personal planning calculators** a la quicken financial planner; see inspiration from [credit karma](https://www.creditkarma.com/calculators/money) and [nerdwallet](https://www.nerdwallet.com/investing/calculators). Monarch also has some interesting calculators you can use even when not logged in like: [1](https://www.monarch.com/calculators/financial-health-calculator), [2])(https://www.monarch.com/calculators/debt-payoff-calculator), [3](https://www.monarch.com/calculators/mortgage-calculator) and probably more if you have an account
  - great free tools with details at [engaging-data](https://engaging-data.com/early-retirement-calculators-and-tools/)
  - [ ] investigate [projection lab](https://projectionlab.com) to understand if that's worthwhile or anything there we want to learn from. from a friend: "really nice stuff built on top of it (roth conversions, drawdown simulation, flex spending, tax strategy, "what if" checkpointing to compare decisions, nice milestone tools to setup when costs are known to change and how, etc"
  - [ ] Investigate [monarch](https://monarch.com) as a suggestion from others
    - ![screencap video](https://stream.mux.com/017Yw7T7QleOQFbHCvL4FHJgZXxbgi003MOxEvUJcqj00I.m3u8)
  - [ ] Investigate [betterment](https://www.betterment.com) which is just a brokerage with up to $8M FDIC, but particularly fo rtheir retirement forecaster, which is pretty simplistic, but also looks useful and something maybe worth modeling on
    - ![screenshot](https://www.betterment.com/hubfs/Webpage%20desktop%20dashboard.png)

## Security
- chore: **cap-sweep residue.** The import preview's `additions` (`import_api::cli_parity`) are still diffed against the config's *forwarded* aliases, capped at `MAX_CONF_ALIASES`, so past 200 it can list a line the write then (correctly) does not add; the write path uses the full list. And "no ASCII control character" now has five private spellings (`accounts`, `periodic`, `aliases` `has_control`; `rules::check_text`; `edit_api::check_text`) — one shared helper.

## Performance
- perf: **`/api/insights` still misses its gate by 1.3×.** It takes 518 ms at 200k against a 400 ms target, and it is the dashboard's landing view. Plan 38 WP-1 brought it down from 916 ms: two `PriceDb` builds instead of eight, one chained replay for both gain windows instead of four, and shared holdings setup with the movers probe. What remains is about 16 full posting passes at 30–60 ms each; the breakdown is in `docs/development/perf-baseline.md` §3. Most of the holdings share is per-posting account classification, which is not memoized: `resolve_account_type` and `resolve_holdings_class` run on every posting in `Currencies`, `stock_accounts`, the sole-symbol facts scan and the replay. The movers probe still runs its own facts scan and replay. Its replay can't be shared because the pools read the price DB, but its facts can (a later facts cap never changes an earlier answer).
- perf: **`value_at` rebuilds the entire `PriceGraph` on every call that misses a direct price.** The graph is a function-local, built and dropped per call, and `graph_at` is O(P). Invisible today because a `$`-only book always hits the direct-lookup fast path — the code's own comment records the cliff as ~6 ms vs ~215 ms. Exposure is per call site: `reports/flows.rs:657` sits inside `for txn in txns` × accounts-in-txn, so **up to 150k graph builds per request**. Also `net_worth.rs:253` (A × D builds) and `holdings/other.rs:350` (per bucket). Fix is to hoist the graph to a caller-owned parameter or memoize it on `PriceDb` keyed by `as_of` — a signature change across ~8 call sites, hence not a quick one. This is the difference between "fine" and "unusable" on any journal that is not single-currency.
- perf: `reachable` (`reports/prices.rs:274`) re-scans the whole edge slice per popped node, so it is O(V·E), **not the O(V+E) its comment at `prices.rs:272` claims**. `shortest_path` enumerates simple paths with a per-partial `Vec` clone and `PriceGraph::rate` runs it twice. Fine on a `$`-star price graph, super-polynomial on a mesh.
- perf: **`latest_directive_price` residual.** The reverse walk exits early only on a base-priced directive, so a symbol that is *never* priced in the base scans its whole prefix — the v2fx corpus's 59 such symbols cost 0.60 ms per series-point against v2's 0.20 ms. The index could answer "this symbol has no base-priced directive at all" and let it exit immediately.
- test: **a third corpus shape with a genuinely DENSE many-to-many FX graph.** This is what the `value_at` / `PriceGraph` finding above actually needs — `v2fx` is a star-of-stars with 2-3 hop chains and low degree, and never exercises `shortest_path`, so that finding is still unmeasured rather than disproved.
- test: `examples/load_rss` takes a transaction count rather than a `Shape`, so there is **no peak-RSS number for v2 or v2fx** — the memory dimension of the 200-commodity shape is unmeasured.

### Front end

The default filter is last-90-days, which is what hides journal-size problems; the cliff is the user's first click on "All time".

- fix(web): **a slow insights refetch shows stale numbers under the new period control, silently.** `createResource`'s `view` is `dataView(status, loaded !== null)` with `matchesRequest` defaulting to true, and `dataView` returns `"data"` when `status === "loading"` and a payload exists (pinned at `loadState.test.ts:17`). Keeping the old data instead of blanking is the right call for perceived speed — but `InsightsDashboard` has **no pending indicator of any kind** (no dimming, no spinner, `PeriodControl` is not disabled), so for the length of the request the boxes say "Last 3 months" while the control says "Last 12 months" and nothing signals it. Wants a subtle refreshing state, not a blank.

