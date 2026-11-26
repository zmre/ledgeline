# TODO


## Known issues
- fix: display issue where pie chart is not round, but oval when the window narrows horizontally or vertically.  Update: seems to be specific to linux as I can't reproduce on mac. Static analysis says the geometry is fine — layerchart computes the radius as `min(width, height) / 2` and emits a true circle with no `viewBox`, and nothing in our CSS scales non-uniformly — so this needs a WebKitGTK repro (dump the `<svg>`'s rendered box, attributes and computed style from devtools on Linux) before anything is changed.

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

## Search

- Our search should line up better with hledger search so you can copy an hledger query in and get the same results
  - This especially means smarting handling of tags when searching
- feat: saved report filters (needs to remember dates as absolute or relative as part of this, not just the search filter)

## Startup metrics (follow-ups to plan 37)

- **Counting commodities (tier D):** headcount (`(kpi:headcount) 1 FTE`), seats, units; revenue per FTE. Find them by a commodity tag, not by name.
- **Inventory metrics:** `kpi: inventory`, inventory turns, DIO, GMROI, sell-through, built on Holdings' lot/cost logic.
- **Churn grace period:** treat a customer as churned only after N consecutive zero months (today N = 1, so a credit month shows as churn + reactivation).
- **Spread-invoices fallback:** for books that can't tag recognition lines, spread each `customer:`-tagged invoice over 12 months (or a `term:` tag) and reconcile against recognised MRR, showing coverage. Plan 45's recognition schedules now post per-customer entries (docs/journal/accrual-recognition.md), which covers books Ledgeline recognises; this stays for books recognised elsewhere.
- **Magic number** and a **cohort retention chart**.
- **Default projection:** extrapolate `kpi: recurring` revenue as run-rate and stop repeating non-recurring revenue.
- **AR aging** (0–30/31–60/61–90/90+), on top of invoice ↔ payment matching (plan 45, docs/journal/invoice-matching.md). Listed under Accrual below.
- **Docs screenshot** of the startup section, if the docs-screenshot harness can serve `fixtures/saas/startup.journal`.


## Accrual (follow-ups to plan 45, its "Phase 5 — Later")

- Insights: renewals by month (count and $, from `renewal-date`), new vs renewal ARR from `contract:`, billed / paid / outstanding per customer from settlement links, and adjusted burn counting prepaid (Δprepaid).
- Invoice matching: N:M settlement, partial payments, AR/AP aging, and customer credit balances from overpayments.
- FX schedules (a schedule in a commodity other than the books').
- Due table: per-row edits of date, amount and accounts before posting, with the "absorb the difference in the final period" prompt (the server has no `edit?`/`absorb?` yet).
- Offer **Match invoices** on an import's or a sync's result screen when it added payments to AR or AP.
- Problems fix buttons for the recognition checks (Delete one, Review restatement, Post as adjustment, Open the rule).
- The Accounts editor's "declare a deferred-revenue / prepaid account" action from the Accrual section.
- Reuse the full transaction body (Phase 3b's Details) in the plan 35 transfer-match overlay.

## Security

## Performance

### Front end

The default filter is last-90-days, which is what hides journal-size problems; the cliff is the user's first click on "All time".

## Invoicer (deferred from the 6c review, 2026-11-25)

- perf: **preview frames reload more than needed** (TW-14). Each draft preview gets a new URL *and* a `{#key}` remount, so after every 400 ms typing pause the frame resets to 1100 px, flashes white and changes height. On the view page every change event reloads the invoice frame and re-fetches detail, books and delivery, even for an unrelated note, though MBR's own live reload already covers the frame. Fix: drop the `{#key}` remount, keep the height across reloads, and filter change events by path.
- fix: **the watcher's outside-journal folders are fixed at startup** (RA-11). A `ledgeline.toml` change rebuilds the session without restarting the watcher, so an `include` added later that points outside the books folder is never watched. Fix: recompute the watched set on `reload_changed`.
- refactor: **Combobox is mostly a copy of AccountInput** (TS-5, `packages/ui/src/components`). Both carry the same popup placement, portal, keyboard handling and listbox markup, and have drifted (AccountInput has Home/End, `onCancel`, `autofocus`; Combobox has the chevron and add-new). Fix: one shared listbox core that both wrap.
