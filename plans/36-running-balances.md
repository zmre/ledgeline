# 36 — Running balances in the journal

Pin an account and the journal table gains two columns: **Change** (what the
row did to that account) and **Balance** (the account's balance after the
row). It is `hledger reg -H <account>` inside the journal view, with its
filters still working. Balance assertions show up inline with ✓/✗, and a
failing assertion can narrow the view to the window where the error must be.

## The ask (Patrick, 2026-11-09)

> Periodically I want to make sure that my balances all line up with account
> statements […] I put in balance checks as the simple way to handle this. But
> when the balance checks fail, I need to track down why and this is where
> things get tricky.
>
> My best tool has been using hledger with running balances, like this:
> `hledger reg -I 'assets:elevations:money-market$'`. That shows me pretty
> quickly where I have the sign wrong on something and it can help me spot
> where there are duplicate entries, too. Also it lets me go through statements
> or activity on an account to scan for where things deviate rather than adding
> more and more assertions.
>
> […] in the journal tab, when looking at balances, if we click on an account
> then we filter to just transactions with that account and we add a column for
> the balance on that account down the side. Further filters (text search or
> date or whatever) can still apply though we need the balance to include all
> transactions […] if the accounts filter were to change (maybe to only show
> dividends), the column would survive even though it could be showing
> transactions that don't include the account we're showing a balance for.
> There would need to be a way to reset back or remove the column, too.

## Decisions (agreed with Patrick 2026-11-09)

1. **The pinned account is separate from the account filter.** Pinning sets
   the filter too, but after that the two move independently. Changing the
   filter keeps the pin. The pin has its own chip, its own ✕ and its own URL
   parameter. **Reset all** clears it.
2. **No filter changes the numbers.** The balance is computed once over the
   whole journal. The date range, account filter and search only decide which
   rows you see. It always includes everything before the visible range
   (hledger's `-H`); there is never a "starts at zero" mode.
3. **Subtrees are included.** Pinning `assets:brokerage` sums
   `assets:brokerage` and everything under it, as the account filter does. For
   a leaf account this makes no difference.
4. **One commodity at a time, chosen with a picker.** It defaults to the
   commodity with the most postings in the pinned subtree. A statement is
   checked one line at a time (cash, then shares of X), and stacking several
   commodities in one cell would make every row tall.
5. **Pinning keeps the current date range.** The balance is correct whatever
   the range, so there is no reason to jump to all dates. (Report drill-downs
   already switch to all dates and keep doing so.)
6. **Two columns, not one.** The transaction's own amount column can show the
   other side (for a dividend reinvestment, the income leg), so **Change** shows
   the pinned subtree's net in the chosen commodity for that row. Without it a
   wrong sign isn't visible.
7. **Rows that don't touch the pinned subtree** (say, after switching the
   filter to dividends) show a blank Change and the balance as it stood at that
   point, dimmed. The column still reads as a timeline, and nothing is hidden.
8. **Quantities as written; costs are ignored.** That's what statements show
   and what assertions check (`assertions.rs`: "assertions ignore costs").
9. **Reconcile mode is out of scope.** Ticking transactions off against a
   statement (using `*` cleared status) is a separate feature, recorded in
   `TODO.md` under Ledger. This plan is the base it will build on.

## Semantics

### Order: the same as the assertion checker

`check_balance_assertions` (`crates/ledgeline-core/src/assertions.rs`) already
orders postings the way hledger does: by **posting** date (a `date:` tag wins
over the transaction's date), with ties in file read order, then posting
order. The running balance **must use the same order**, or the column could
contradict a ✓/✗ on the same row. Move the entry-building and sort into a shared
function rather than writing it twice. hledger's `reg` uses this order too, so
the parity test below can compare directly.

The table displays rows by transaction date descending, then `tindex`
descending. Within a day that's the exact reverse of read order, so the
newest-first column reads correctly from top to bottom. **Check that
`tindex` order equals position order** for `include`d files before relying on
it, and pin it with a test.

### What each row shows

Let *S* be the pinned subtree's postings in the chosen commodity, in the order
above, each carrying the running total after it.

| Row                                     | Change                              | Balance                                                       |
|-----------------------------------------|-------------------------------------|---------------------------------------------------------------|
| Has postings in *S*                     | Their sum                           | The total after its **last** posting in *S*                   |
| Touches the subtree, other commodity    | Blank                               | Carried (dimmed), as in the next line                         |
| Doesn't touch the subtree               | Blank                               | Carried: total after every entry keyed ≤ (row date, position) |

A carried value is found by binary search over *S*.

**Posting-dated legs.** A transaction dated 03-28 whose bank leg carries
`date:2026-04-02` puts that leg into the stream on 04-02. The row still sits
at 03-28 in the table. Its Change cell says "posts 2026-04-02", and its Balance
is the total after that leg, exactly what `hledger reg` prints on that line.
The column can then be non-monotonic for those rows. That's correct, and the
label explains it. It is rare enough that it isn't worth more.

### Assertions in the column

A row whose posting in the pinned subtree carries an assertion shows a mark in
the Balance cell:

- **✓** when the assertion holds.
- **✗ expected $12,400.00, off by −$250.00** when it fails.

Pass or fail is decided **by the assertion checker**, not by comparing the
assertion to the column. They differ legitimately: a plain `=` on a child
account asserts that child's own balance, while the column sums the whole
pinned subtree. When the asserted account isn't the pinned root, the mark names
it ("✓ checking:joint"). Failures come from `AssertionFailure` (`asserted`,
`calculated`, `transaction_index`, `position`). Everything else that asserts is
a pass.

Only assertions in the chosen commodity are marked, except a `==` failure on
another commodity, which is always shown: it means a commodity that should be
zero isn't.

## Engine (`crates/ledgeline-core/src/view/running.rs`, new)

- `posting_order(journal) -> Vec<PostingRef>`: the shared order, moved out of
  `assertions.rs` (which then calls it). `PostingRef` is `(position: u32,
  posting: u16)` plus its effective date.
- `RunningBalance::build(journal, order, root: &str) -> RunningBalance`: one
  pass over `order`, keeping postings where `account_matches(root, account)`.
  For each commodity it builds `Vec<Entry { date, position, change, after }>`.
  It also counts postings per commodity, for the picker's ranking.
- `RunningBalance::cell(commodity, row_position, row_date) -> RunningCell`
  returns `{change: Option<Dec>, balance: Dec, touches: bool, posted_on:
  Option<&str>}`. Uses a lookup from position to entry range for rows that
  touch, and binary search otherwise.
- `RunningBalance::assertions(commodity, failures) -> Vec<AssertionMark>`: for
  every asserting posting in the subtree, `{position, account, operator,
  asserted, calculated?, ok}`, in stream order. The gap feature needs this list
  in order.

Tests (core, no server):

- Leaf, subtree, multi-commodity; a row that touches only another commodity; a
  row that doesn't touch the subtree at all.
- Same-day ties in read order. `include`d files. Posting-dated legs, including
  two legs of one transaction with different `date:` tags.
- Virtual `(a)` and balanced-virtual `[a]` postings count, as they do for
  assertions. Generated (`--auto`) postings count when the snapshot has them.
- Costs ignored. A zeroed commodity remains in the list.
- **hledger parity golden.** For a fixture journal, the `(change, balance)`
  sequence equals `hledger reg -H -O csv '<root>' cur:'<c>'`, captured once into
  `fixtures/journal-view/` like the other goldens. Don't regenerate it from
  Rust.
- **Agreement with assertions.** For every passing assertion with `=*` on the
  pinned root, the asserted figure equals the column's balance on that row. A
  property test over generated journals would be ideal.

## Server (`crates/ledgeline-server/src/journal_api.rs`)

### Cache

- `ViewCache.posting_order: OnceLock<Arc<Vec<PostingRef>>>`, warmed lazily (not
  in `prewarm`: most sessions never pin).
- `ViewCache.running: Mutex<Vec<(String, Arc<RunningBalance>)>>`: an LRU of 4
  roots, in the same pattern as `filters`. One build covers every commodity, so
  switching the picker costs nothing.
- `ViewCache.assertion_failures`: the raw `Vec<AssertionFailure>` from the one
  engine pass the diagnostics already make. Keep it rather than running the
  check a second time. If the diagnostics only keep `Finding`s today, keep both
  from the same run.

At 200k transactions (~600k postings), the scan plus per-commodity vectors
should come in at a few ms. Record it in `docs/development/perf-baseline.md`.

### `GET /api/journal/rows`: two optional parameters

`balance=<account>&balanceCommodity=<c>`. When `balance` is present, every row
gets:

```jsonc
"running": {
  "change": {"m": "-25000", "p": 2} | null,  // null: no posting in the subtree in this commodity
  "balance": {"m": "1240000", "p": 2},
  "touches": true,
  "postedOn": "2026-04-02",                  // only when it differs from the row's date
  "assertions": [{"account": "assets:…", "op": "=", "asserted": {…}, "calculated": {…}, "ok": false}]
}
```

The amounts use the commodity's declared style, like the rest of the row. If
`balanceCommodity` is missing, the server uses the default (most postings).
An account that matches nothing is `400`, not an empty column. When `balance`
is absent the response is byte-for-byte what it is today.

### `GET /api/journal/running?account=<root>`, new

Feeds the chip and the commodity picker:

```jsonc
{
  "generation": "…",
  "account": "assets:elevations:money-market",
  "commodities": [{"commodity": "$", "postings": 412}, {"commodity": "VTI", "postings": 37}],
  "assertions": {"$": [{"tindex": 812, "date": "2026-03-31", "ok": false}, …]}
}
```

The ordered assertion list per commodity is what "narrow to the gap" and "first
failure" run on, without paging through rows. ETag, gzip and the report pool
work like the other routes.

Add both to the routes table in `docs/development/journal-view.md`. **Run `just
test-integration`** after the change (the unit suite skips the live-engine
tests).

## SPA

### State and URL

- `stores/filters.svelte.ts`: the filter gains `pin: {account, commodity |
  null} | null`. `reset()` clears it. Setting accounts, dates or the query never
  touches it.
- `filters/urlCodec.ts`: `bal=<account>` and `balc=<commodity>`. A missing
  `balc` means "server default", so a shared link stays short. Round-trip tests
  go next to the existing ones.
- `stores/journalView.svelte.ts`: the block cache key includes the pin, so
  pinning, unpinning or changing the commodity drops blocks the same way a
  filter change does. A new `running` resource follows (server, generation,
  pin.account) for the chip.

### The chip

It sits in the active-filter row, visually distinct from the filter chips (it
changes what the rows show, not which rows):

```
⚖ Running balance: assets:elevations:money-market  [$ ▾]  ✗ first failure 2026-03-31  ✕
```

- `[$ ▾]` is the commodity picker, shown only when the subtree has more than
  one commodity.
- "✗ first failure …" appears when any assertion in this subtree and commodity
  fails. Clicking it scrolls to that row (the same mechanism the Problems
  drawer uses), widening the date range only if needed.
- ✕ removes the pin and the columns. The account filter stays as it is.

### Columns

`TransactionTable.svelte` adds **Change** and **Balance** to the right of the
amount column while a pin is set. They are right-aligned and use tabular
figures. A negative Change uses the same colour as other negatives. A carried
Balance is dimmed (`text-base-content/50`). Assertion marks sit inline after
the balance, with full detail in a tooltip. The skeleton rows reserve both
columns so the layout doesn't jump.

### Entry points

- **Balances view**: clicking an account row (or pressing Enter on it) sets
  `accounts = {account}` and pins it. The date range doesn't change. Rows
  aren't clickable today; give them the same keyboard and focus treatment as
  report rows.
- **Account filter chips**: each account chip gets a small ⚖ button that pins
  that account and keeps the filter. This is how you pin from inside the
  journal.
- **Reports and Holdings**: Enter on an account already opens the journal
  filtered to it over all dates. Add `bal=` to that link so the column comes
  with it.
- **Keyboard**: a pin/unpin shortcut if a key is free. Check `keyboard.md` and
  the help overlay, and add it to both.

### Narrow to the gap

A ✗ mark has a **Narrow to gap** action. It sets the date range from the day
after the previous *passing* assertion in this subtree and commodity (open if
there is none) to the failing assertion's date. The account filter and search
are left alone, matching the Problems drawer. The ordered list comes from
`/api/journal/running`.

### Export

When a pin is set, the CSV/XLSX export (`export/journalXlsx.ts`) adds Change
and Balance columns. Export is documented as "exactly what the filters show",
and with a pin, that includes the columns.

## Suspects (phase D)

Within a gap, let *d* = calculated − asserted. Mark rows (and say why) in the
gap window whose Change *c* in the chosen commodity satisfies:

- **c = d / 2**: "sign flipped?" A posting entered as +x that should be −x is
  off by 2x.
- **c = d**: "duplicate or extra?" The badge is stronger when another posting
  in the window has the same Change within 7 days.

Use exact decimal equality, no tolerance. These are hints, never edits.

There's also a **Search for this amount** button on the ✗ tooltip that puts
|d| (and, as a second button, |d/2|) into the search box. Search already
matches amounts as displayed, so this finds the matching posting **in another
account**, which is how you spot an entry booked against the wrong account.

This phase is separate because it's the most speculative part. Ship A–C, use it
on a real reconciliation, then decide how much of D is worth it.

## Phases

| Phase | Scope                                                                                   | Done when                                                               |
|-------|-----------------------------------------------------------------------------------------|-------------------------------------------------------------------------|
| A     | Engine: shared `posting_order`, `running.rs`, tests, hledger parity golden               | Core tests green, parity golden matches, assertions suite unchanged      |
| B     | Server: cache entries, `rows` params, `/api/journal/running`, decode + types in the SPA  | Route tests, `just test-integration` green, perf number recorded          |
| C     | SPA: pin state + URL, chip + picker, columns, entry points, assertion marks, gap, export | Unit + component tests, e2e for pin → filter change → column survives   |
| D     | Suspects and search-for-amount                                                          | Tried on a real failing assertion; kept, trimmed or dropped              |
| E     | Docs: `docs/journal/running-balances.md` + screenshot, filtering.md, keyboard.md, dev doc | Docs build, screenshot generated by `web/docs-screenshots`             |

## Open questions to settle during the build

- Whether the Change column also shows when a row has postings in the subtree
  but nets to zero in the chosen commodity (e.g. a transfer between two child
  accounts of the pin). Proposal: show `0.00` with "internal" in the tooltip,
  since it is a real touch.
- Width: on a narrow window, two more numeric columns may squeeze the
  description. Probably drop Change before Balance, but decide by looking at it.
