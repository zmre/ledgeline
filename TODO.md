# TODO


## Known issues
- fix: display issue where pie chart is not round, but oval when the window narrows horizontally or vertically.  Update: seems to be specific to linux as I can't reproduce on mac. Static analysis says the geometry is fine — layerchart computes the radius as `min(width, height) / 2` and emits a true circle with no `viewBox`, and nothing in our CSS scales non-uniformly — so this needs a WebKitGTK repro (dump the `<svg>`'s rendered box, attributes and computed style from devtools on Linux) before anything is changed.

## Security
- security: **a local process can read the token straight out of `GET /`, and that escalates to
  code execution.** `PUT /api/prefs` persists an arbitrary `hledgerPath`, which `Hledger::candidates`
  then spawns on every import.
  - The real fix is to hand the token to the WebView **out of band** — wry's
    `with_initialization_script` — so the token never appears in a response body at all. `GET /` is
    deliberately outside `route_layer` so the browser can bootstrap, and HTTP cannot tell the user's
    WebView from someone else's `curl`.
  - Needs care: the vite dev flow and the Playwright e2e harness bootstrap cross-origin and would
    have to keep using `$LEDGELINE_TOKEN`.
  - The prefs validation only removes the *persistent* half. The `--version` probe is itself an
    `exec`, so a caller can still run an arbitrary binary by repeating `PUT /api/prefs` — measured
    running the candidate **twice** per request, which is worth fixing on its own since one would do.
    (Permissions are checked before the probe, so a group/world-writable binary is never run.)
- security: **an xlsx with two cells can ask for a ~550 GB allocation.** `convert/spreadsheet.rs`
  lets calamine materialize a dense grid from sparse bounds (`A1` + `XFD1048576` is enough).
  `MAX_INPUT_BYTES` bounds the *compressed* zip and `MAX_ROWS` truncates only *after*
  materialization, so neither helps. An allocator abort is not something `CatchPanicLayer` can
  catch. Needs a streaming/bounds-checked read rather than a cap.
- security: **TOCTOU between the symlink refusal and the write.** The refusal uses
  `symlink_metadata` but the write follows symlinks, and the window spans up to three 120-second
  `hledger` runs. Worse on the edit path, where `carry_forward_metadata` copies the victim file's
  uid/gid/mode onto the new inode and erases the trace. Wants open-once-and-write-through-fd.
- security: **the CSP inline-script hashes are never tested against a real SvelteKit shell.**
  `flake.nix` injects a script-less placeholder for every CI test and Playwright runs cross-origin
  with no CSP, so a SvelteKit upgrade can break the app to a blank window with nothing failing —
  and the obvious pressure fix would be `unsafe-inline`. Wants a fixture built from a real shell.
- security: `wire.rs`'s `/accounts` attributes an `include`d file's line number to the main journal
  (`adisourcepos` is built with the main journal's name regardless of which file the directive came
  from). `AccountDeclaration` carries a `SourcePos` but no `source_file`, unlike `Transaction` and
  `AliasDirective`, which carry both. Blocks file+line anchoring for directive diagnostics.
- chore: **sweep for caps that exist but are applied inconsistently.** Three were found one at a
  time (a render clamp present in `edit.rs` but missing from `ofx.rs` and `import_api.rs`, a parse
  scale cap that failed open, a row cap that ran after materialization). Worth looking for the rest
  deliberately rather than waiting for the next review.
- chore: **one `no_store` definition, not five.** The canonical `HeaderValue` is in
  `security::no_store()`, but `import_api`, `rules_api`, `alias_api` and `budget_api` each still
  have a module-private `fn no_store<T: Serialize>(body: T) -> Response`. Rebase them onto it.
- chore: **one path redactor, not two.** `import_api::Redactor` is private to that module, so
  `edit_api::PathRedaction` duplicates its longest-first and both-spellings-of-`/tmp` rules. Promote
  `Redactor` to a shared module and delete the duplicate.

## Misc
- chore: Add screenshots and better descriptions to the readme
- ledgeline looks great, but it inherits the complexity and issues of web apps due to our architecture choices
  - these were the right choices for mbr (cuz markdown) but maybe not here
  - what if we used <https://iced.rs> or [GPUI](https://github.com/longbridge/gpui-component) or something? libcosmic? [freya?](https://github.com/marc2332/freya)
  - for forms and displays of numbers and such, it would probably be a great improvement
  - for charts, i expect we'd be in trouble; egui has some libraries that might do

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
- feat: quickbooks import handling
  - transaction matching and skipping
  - account mapping (prompt for unmapped or use aliases?)
- feat: create new import rules files
  - take a csv file and make intelligent guesses on setup. we want intelligent mapping of headings, ask what account it is and default categorizations, figure out ordering of rows. detect separator, skip rows number, and encoding automatically. figure out date-format automatically. 

## Editors
- Account List Editor
  - Most financial apps allow editing of the chart of accounts. We should detect where they live and allow editing. If there aren't any, we should create an accounts.journal and include it from the main file.
  - For each account, we should provide an editor for comments/notes, type, tags in general, and our special tags used in various reports
  - Lets put this under a Settings top level tab or gear icon. And lets figure out what else might go in here, like the number format stuff -- basically whatever hledger provides that we might want to set or edit
    - commodity, decimal-mark, tag list, and we should probably move aliases to here under "settings" too

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
- feat: gain timeline improvements
  - when i change the gain timeline, everything else should update, too, notably the "value over time" which is fixed to previous 12 months
  - the gain timeline also needs more options. lets do 5yr, 3mo, 1mo, and 1 week as additions
- feat: in holdings tab, optional compare performance to S&P, Dow, Nasdaq, Bonds, US Stock Index
  - Idea is to be able to see how one's portfolio is doing versus just standard options where each can be checked or unchecked.
  - Our current line chart is value (dollars) over time and we need to overlay basically what it would have been like if every investment was made at the same time with the same money but into X as a separate line (dotted and different color) to show the comparison and difference in the chart.
  - We don't need this per stock and we don't need it as an overall percentage -- just the line chart.
- feat: more pie chart views of stocks
  - if yahoo gives us more than just price information then it might be nice to have a drop down on our stock holdings pie chart to show it divided up in a few ways: risk categorization, asset class, security type, industry, etc. -- whatever we can get -- then we show the pie chart instead of by investment, by category
  - we would need a way to have a tag on commodities that could specify (or override even) this information
  - if yahoo doesn't provide this information for free, then we'd need to see if there's another place that does. there's no server, so any sort of signup or account requirement is a blocker. we'll skip this feature if we don't have a data source.

## Ledger 
- feat: intelligent category suggestions
  - only real way to do this is with some sort of lookback comparing similar descriptions in the past and seeing associated expense or revenue accounts
  - need to remove random numbers from description and maybe do a predominance calculation or a vector comparison rather than full equality.  if we're doing equality and removing numbers, we need to normalize some by lowercasing.  but in a perfect world, "netflix.com" might see a previous "netflix" and guess category based on that.  the more exact the match and the more recent, the higher the sort ranking
  - feat: remember categorization functionality — write a chosen category back into the rules file as a new `if` rule (the rules editor and its write path are done; this is the one-click path into them)
- feat: File -> New
  - Here I'm assuming we're setting up a new set of journal files, chart of accounts, etc. Probably we prompt with some questions and use an empty folder as a starting point and then create a skeleton so someone can start using us to track things. We should have a default chart of accounts for individuals and another for businesses and then we should allow them to start with an empty set of accounts to add their own.
- feat: saved report filters?

## Budgeting / planning / modeling
* budget fix revenue display: There's something wrong when a budget shows you red for earning more money than expected. We need to treat revenues/income/sales/whatever differently from expenses in terms of how we present current state to the user. If revenue is under budget (and not on-track -- so if we're looking at annual budget but we're only on month six, then on-track would be half of annual budget) then the whole line should be red. If we're "over budget" meaning we've made more than we budgeted, then the whole bar should be green (with the target white line showing appropriately).
* budget projections: Seeing if you're above/below budget though really doesn't tell you much. Ultimately we need to be able to project into the future to understand the what-ifs.  Given the current budget's income and expense information, what net income is bing projected going forward?  I see this as a new section. So we have actuals vs. budgeted graphics in the top, then the budget goals below that, then below that we should show a very simplified sort of P&L summing up inflows, outflows, and net income using red/green to indicate health.  Ultimately we'll get far more sophisticated on projections, but it's worthwhile to have something basic here to start.  I'm not sure if it's worthwhile to project balances forward, too, maybe summed by starting with cash and then either adding to it or subtracting from it over time?  This would be useful as a sort of napkin-level runway check (if losing money) or savings check (if cash flow positive) as a validation on budget.  This way someone can know they have to tighten one budget or another.
* budget gaps: As a last point on budgeting, I wonder if it wouldn't be worthwhile to show unbudgeted income and unbudgeted expenses to give the user some idea of how encompassing their budget is versus historical income and expenses and maybe tell them what higher-level categories are being ignored. It may be fine to ignore these (we have set budgets for some things like clothing, but not for other things, like insurance) so this section should be expandable and default to being collapsed
* While we're at it, the budget goals are currently sorted however they are in the budget file which is whatever order they happened to be added in. So I have revenues:salary at the top and revenues:dividends at the bottom, which makes it hard to scan.
- feat: personal planning calculators a la quicken financial planner; see inspiration from [credit karma](https://www.creditkarma.com/calculators/money) and [nerdwallet](https://www.nerdwallet.com/investing/calculators)
  - great free tools with details at [engaging-data](https://engaging-data.com/early-retirement-calculators-and-tools/)
  - TODO: investigate [projection lab](https://projectionlab.com) to understand if that's worthwhile or anything there we want to learn from. from a friend: "really nice stuff built on top of it (roth conversions, drawdown simulation, flex spending, tax strategy, "what if" checkpointing to compare decisions, nice milestone tools to setup when costs are known to change and how, etc"

  - in business, I also build modeling spreadsheets that allow for what-ifs and let me model sales growth, investments, ramp up in hiring/expenses, etc., so i can understand runway (cash balance over time anyway)
    - It would be great if we had a mechanism for unifying this
  - we need to understand how forecasting works in hledger -- is it just the budgeting? can we use that for forecasts meaningfully?
    - partly answered by the budget editor (`plans/15-budget-editor.md`): hledger's `--forecast` reads the
      SAME `~` periodic rules `bal --budget` does, so `ledgeline-core/src/periodic.rs` — the span editor
      the budget tab writes through — is already the document model a forecast scenario would be written
      with. It is named `periodic` rather than `budget` for exactly that reason.
  - fix: our parser REJECTS every period expression outside the five fixed intervals — `~ every 2 weeks`,
    `~ monthly from 2026-01 to 2027-01` — and the rejection fails the WHOLE journal parse, not just the
    rule. Forecasts need those forms (a scenario is bounded by definition), and today a user who writes
    one cannot open their journal at all. `parse::parse_period_expr` + `model::PeriodExpr` are where it
    lives; `periodic::BlockLock::Period` already presents an unmodelled period read-only, so the editor
    side is ready for whatever the parser learns to read.
  - i want to be able to save different forecasts, probably as files not included in the main, but which we can read in and apply and then produce reports into the future (balance sheet, p&l, cash flow, etc).
  - we should be able to do things with percentages, too, so we can assume some percentage return on investments or increase in salaries, expenses, and so on.  Not sure if we'd have to calculate this and save it out or if we could bake it in using hledger's auto postings
  - I really want this feature to be a nice GUI but then something that persists and still works with hledger to the greatest extent possible.
  - We should be able to have pre-canned report templates where there are sliders that for indicating certain things and they produce the forecast files that we then show based on those values (which could be captured as comments if needed so we can edit and change easily later).
    - The idea here would be for a basic personal one with questions about how many years until retirement, how much invested into retirement accounts, assumptions on stock returns and inflation, assumptions on spending per year in retirement, taxes, what income streams will continue (dollar amount and adjustment per year) in retirement, etc.
    - There would be different questions for business ones and other ideas, but the core here is that a user could do absolutely anything, but we make it really simple to setup some basic forecast scenarios.
