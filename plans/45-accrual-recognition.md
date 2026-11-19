# 45 — Accrual recognition: deferrals, prepaids, invoices and their schedules

Ledgeline gains a set of helpers for accrual-basis books: setting up and
posting revenue and expense **recognition schedules** (deferred revenue,
prepaid expenses), and **matching invoices and bills to their payments**
through receivables and payables. A cash-basis or household ledger sees none
of it.

The schedule of a contract is plain hledger: `~` rules in a separate
`recognition.journal` that the main journal does **not** include. Each month's
recognition entry is **materialized**: written into the main journal as an
ordinary transaction, after the user approves it (or automatically, if they
opt in). Nothing is ever posted for a date after today. Each source posting,
rule and posted entry carries one `recognition:` id, so hledger can query the
whole contract and Ledgeline can tell which periods have been posted.

## The ask (Patrick, 2026-11-17)

> Make entry, editing and tracking of revenue and expense recognition far
> easier than QuickBooks, where the monthly recognition JEs are entered by
> hand. … fast, effective, makes a bookkeeper faster; performance matters on
> large ledgers.

Only for accrual-basis ledgers: "cash-basis and household ledgers see
nothing".

Revenue, in four shapes:

1. **Paid and delivered at once.** `revenue:sales` against the bank, with
   `customer:` posting tags. The only help needed is asking for the customer.
2. **Prepaid, no invoice, recognized over a term** (an annual SaaS prepayment
   by card):

   ```journal
   2026-11-17 Customer X Payment
        liabilities:deferred-revenue    $-12000    ; customer: Customer X, term-months: 12
        assets:bank:checking             $12000    ; customer: Customer X
   ~ monthly from 2026/12/17 to 2027/11/17   Customer X Recognized  ; renewal-date:2027-11-17
       liabilities:deferred-revenue     $1000    ; customer: Customer X
       revenue:sales                   $-1000    ; customer: Customer X
   ```

   > User indicates prepay-over-time → ask term → ask target revenue account →
   > suggest deferred account(s). On renewal, prefill from the customer's
   > previous contract … editable (e.g., switched annual→quarterly).

   A payment can have several lines: fees (never spread), one-time items (not
   deferred), and several deferred lines. "Key off each line hitting deferred
   revenue." One payment generally gives one recurring entry, even with
   several deferred items. If their terms differ, the plan must say what
   happens.
3. **Invoice, then payment, then recognition.** The invoice debits
   `assets:accounts-receivable` and credits deferred revenue. The payment
   clears AR to the bank. Recognition starts at the service start date, even
   if the invoice is unpaid, "ideally on the monthly anniversary of the start
   date". Invoice and payment are matched with AR treated as a clearing
   account. Fees can make the amounts differ.
4. **Billed after delivery.** The invoice credits revenue directly, and the
   payment clears AR. Nothing is deferred.

The expense side mirrors this: accounts payable for bills, and prepaid
expenses (annual D&O insurance) recognized monthly into, say,
`expenses:insurance`, with `vendor:` tags:

> today different insurance policies with different start dates are rolled
> together in one JE, which is confusing.

An external invoicing app may write the invoice entries. Ledgeline still has
to spot the payment, match it, and set up the recognition.

Periodic entries:

> optionally auto-materialize (user preference), otherwise prompt via a button
> like the existing review/matches indicators; approve with adjustments; if
> books not done for months, catch up all missing. NEVER materialize future
> periods — only past … Must detect already-materialized to avoid duplicates.

Restatements: if a source entry is edited in Ledgeline, propose updating the
rule and everything already posted from it. Edits made outside Ledgeline are
the user's responsibility.

The purpose is books that are easier to get right and keep right, plus data
to mine: renewals by month and dollar amount, new vs renewal (for commissions
and residuals), and billed vs paid at a glance, with no spreadsheet on the
side. Customer and vendor tags are asked for but optional.

The known downside is one recognition entry per customer per month, which is
clutter at consumer scale. There should be a quick way to hide those entries
while they still count in reports and Insights.

Finally: billing that's anchored to the 1st, with a prorated first period.
"How do we handle it?" That's answered in D-anchor.

## Decisions (Patrick, 2026-11-17)

| #  | Decision                                                                                                                                                                                                                                                              |
|----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| D1 | Recognition `~` rules live in a **separate file the main journal does not include** (`recognition.journal` beside it by default). Creating it is an explicit action. Materialized entries are real transactions in the main journal.                                 |
| D2 | Recognition rules **never** reach projections or the Budget tab. A user who wants them in a projection copies the file.                                                                                                                                               |
| D3 | Proration: equal amounts per period, any rounding remainder in the final period (true-up). Billing anchored to the 1st gets a first stub prorated by days. Stubs and true-ups are single-date `~ YYYY-MM-DD` rules beside the main rule. Recognition is monthly, even for quarterly or annual terms. |
| D4 | Restatement: an edit to a source entry made in Ledgeline proposes rewriting the rule and every entry already posted from it, **except** entries on or before `closed-through`. Those get one catch-up adjustment in the current month instead.                          |
| D5 | Invoice↔payment matching v1 is 1:1, by customer/vendor, invoice number and amount within a fee tolerance, with the difference offered as a fee posting. It reuses plan 35's `match:` tag and engine, with AR/AP as the clearing accounts. N:M and aging come later. |
| D6 | Catch-up: a batch table grouped by month, then by customer/vendor. Every row starts checked. Per-month totals tie to the deferred/prepaid balance. A row expands to edit. **Post through ‹month end›** writes them all, and never posts past today. Auto-materialize is opt-in (off by default). |
| D7 | Gate: the helpers appear when an account declares `kpi: deferred-revenue`, `receivable`, `payable`, or a new `kpi: prepaid`. Failing that, a name is inferred. `basis = "cash"\|"accrual"` in `ledgeline.toml` overrides both. A declared type or `kpi:` always wins over a name. |
| D8 | The journal list collapses posted recognition entries into one summary row per month, which expands in place. A filter chip hides them entirely. They always count in reports and Insights.                                                                     |

## What hledger 1.52 actually does

Each fact below was verified on 2026-11-17 against scratch journals in
`/tmp/p45`, using `hledger` 1.52 from the dev shell
(`/nix/store/zslir4fj0h102gl0030z1b4i5bgpazq5-hledger-1.52`).

1. **`to` is exclusive.** `~ monthly from 2026-12-17 to 2027-11-17` gives
   **11** occurrences (Dec 17 … Oct 17), not 12. `to 2027-11-18` gives 12.
   Ledgeline always writes `to` as the day after the last occurrence it
   wants (see D-sched). Ledgeline's own model already says so:
   `model.rs:469-471`, `PeriodSpec.end` "EXCLUSIVE".
2. **`monthly from D` anchors on D, clamped.** `~ monthly from 2027-02-28`
   fires on the 28th every month after that. A contract starting on Jan 31,
   with its first recognition on Feb 28, would drift to the 28th. `~ every
   31st day of month from 2027-02-01` gives Feb 28, Mar 31, Apr 30. So any
   start day from 29 to 31 is written in the `every Nth day of month` form.
   Ledgeline's period engine already reads that form
   (`PeriodKind::Anchored`, `model.rs:429`; `budget.rs:222`).
3. **Single-date rules work, and combine with the others.** A file holding
   `~ 2026-11-30` (stub), `~ every 31st day of month from 2026-12-01 to
   2027-11-01` and `~ 2027-11-16` (true-up) forecasts exactly those
   occurrences. `bal tag:_generated-transaction` sums to the contract amount.
4. **Every `~` rule is a budget goal.** `bal --budget -f main -f budget -f
   recognition` shows the recognition rules as goals beside the household
   goals. `-R` drops the virtual-posting goals (the household budget) and
   **keeps** the real-posting ones, the recognition rules, which is the
   opposite of useful. `--budget=household` restricts goals by description.
   So the recognition file must never sit where budget goals are read (D1).
5. **`--forecast` reads `~` rules from every `-f` file.** No flag forecasts
   one file and not another.
6. **Forecast transactions carry the rule's tags.** A generated transaction
   has the rule's header comment tags (`recognition: r1, renewal-date: …`),
   its posting tags (`customer: …`), the hidden `_generated-transaction`, and,
   with `--verbose-tags`, a visible `generated-transaction: ~ <period>`. The
   query `expr:"not (tag:recognition and tag:_generated-transaction)"` keeps
   the posted entries and drops the generated ones.
7. **When the forecast starts: per file.** A bare `--forecast` starts after
   the latest ordinary transaction *in the file that holds the rule*, or at
   **today, inclusive**, when that file has none. Verified in `/tmp/fc2`: the
   same rule `from 2026-02-17` with real transactions through 2026-03-05
   forecasts from 2026-03-17 when everything is one file, but from 2026-11-17
   (today) when the rule sits alone in `-f r.journal` beside `-f m.journal`.
   `recognition.journal` holds only rules, so with the D1 layout the forecast
   always starts today, whatever the main journal's latest date. With
   `--today 2027-03-17`, the 2027-03-17 occurrence is generated. The
   consequences:
   - Unposted past periods disappear from `--forecast` output. That's the
     reason to materialize.
   - An entry posted **for today** is counted twice by a bare `--forecast`.
     The docs therefore give `--forecast=tomorrow..`: with an explicit start,
     the forecast begins at the later of the two starts. It was verified to
     begin with the next occurrence after today.
   - An explicit past start (`--forecast=2026-12..`) regenerates every past
     occurrence beside the posted entries, so they double. The screening
     query in fact 6 is the fix.
8. **Strict mode across sibling files.** `hledger -s -f main -f recognition
   check` passes when `main` declares the accounts. Sibling `-f` files don't
   share `commodity` or `decimal-mark` directives (already noted at
   `parse.rs:470-478` for projections). So Ledgeline writes the recognition
   file's amounts in a form that reads the same on its own (see "The
   recognition file").
9. **Plain reports ignore `~` rules.** `hledger -f main.journal bal` (and
   `-f main -f recognition` without `--forecast`) shows exactly the posted
   entries. That's why posted entries are real transactions.

## What is true today (in the code)

- **Classification by `kpi:`.** `reports/kpi.rs:29` `KpiRole` has
  `DeferredRevenue`, `Receivable` and `Payable`, but no prepaid role.
  `kpi.rs:145` `inferred_role` infers only receivable and payable from names,
  and its doc says "Nothing else is ever inferred from a name". The gate
  (D7) needs deferred and prepaid inference too. That inference is added for
  the gate only (see D-gate), so Insights' plan 37 behaviour doesn't change.
- **`~` rules are budget goals by construction.** `model.rs:733`
  `Journal::periodic_transactions` is "consumed only by the budget report".
  The budget report (`reports_api.rs:4178`), budget gaps (`:4217`), the
  budget editor (`budget_api.rs:583`, `:717`) and the projection seed
  (`projections_api.rs:2120`, the default projection of plan 33) all read
  that one vector. A recognition file that never enters it is invisible to
  all four. So D1 (no `include`) is also the implementation of D2, provided
  the rules are parsed into a **separate** structure.
- **Parsing a sibling file with the main file's styles already exists.**
  `parse.rs:482` `parse_journal_with_styles` is what projection scenarios
  use.
- **Occurrence generation is shared and matches hledger.** `budget.rs:215`
  `occurrences(start, end, spec)` handles `Every`, `Anchored`, `Annual` and
  `Once`, applying the exclusive `to`.
- **Forecast detection is tag-based.** `projections/forecast.rs:385`
  `is_generated` is true for any transaction tagged `generated-transaction` or
  `_generated-transaction`. Review (`review.rs:39`) and matching
  (`matching.rs:29`) both skip such transactions. **A posted entry must never
  carry either tag**, or it would vanish from review, matching and the
  latest-real-date logic.
- **Writes.** `edit.rs:1087` `add_transactions` adds N transactions with
  **one** reparse, placed by `InsertPosition::DateOrdered` into the file
  that holds their chronological neighbours (`edit.rs:972-976`). That's the
  right writer for posted entries. `edit.rs:2009` `apply_batch` and
  `POST /api/transactions/batch` (`review_api.rs:1242`, `MAX_BATCH` = 1,000
  at `:99`) do fingerprint-checked surgical edits: `set_posting_tag`
  (`edit.rs:1908`), `add_posting_tag_value`, `set_txn_tag` (`:1821`).
- **The budget editor's file pattern** (`budget_api.rs:1306`
  `create_budget_file`) writes the new file first and then appends an
  `include`. The recognition file reuses the first half and **skips** the
  `include`.
- **Matching** (`matching.rs`):
  - `clearing_accounts` (`:351`) means accounts tagged `clearing:`.
  - Proposals (`propose`, `:1235`) need exactly opposite amounts within a
    10-day window.
  - `LinkIndex` reports a `match` id on a non-clearing posting as
    `BrokenReason::NotClearing` (`:186`, `:1459`).
  - `view/rows.rs:347` `Transfers::of` collapses **every** valid
    two-posting link into a transfer row.

  An invoice↔payment link through AR would therefore be "broken" today, and
  even once valid it would collapse into a transfer row. Both need a change
  (Phase 1).
- **Journal rows are collapsed in the engine.** `journal_api.rs:90` caches
  `Transfers` per snapshot. `journal_api.rs:356` applies `collapse` to every
  filter result. `total`, offsets and positions count rows, not transactions
  (plan 35 D11). Recognition grouping goes in the same place.
- **Review.** `ReviewOverlay.svelte` is both the add and the edit card
  (`journal/edit/modalState.svelte.ts:13`, `TxnModalMode = "add" | "edit"`).
  Its collapsible "Similar unreviewed" section is `SimilarSection.svelte`,
  mounted at `ReviewOverlay.svelte:1158-1180`. The toolbar buttons (Review
  Unknowns, Review Recents, Find Matches) are in
  `journal/TransactionTable.svelte:862-890`.
- **Subscriptions** (`reports/subscriptions.rs:260` `expense_charge`) look
  at the expense side of each transaction and cluster by payee, amount and
  cadence. A monthly prepaid-insurance recognition entry, debiting
  `expenses:insurance` by the same amount every month under one description,
  **would be detected as a subscription**. Posted entries must be excluded.
- **Settings.** `ui_prefs.rs:10-30`: UI preferences go to the global
  `ui-prefs.toml`. `ledgeline.toml` is written only for keys it already sets,
  and is created only for an explicit theme pin. `ui_prefs.rs:1348`
  `edit_ledger_text` is the second exception, `[sync]` (plan 43). The module
  doc (`:33-37`) says `ledgeline.toml` "may hold UI preferences ONLY", so a
  `[books]` table is a third, documented exception (D-settings).
- **Live reload.** `lib.rs:794` `watched_files` is `journal.source_files`
  plus the conf files. A file the journal doesn't include is not watched
  today, so the recognition file has to be added.
- **Projection discovery lists every `*.journal` under the journal's
  directory** (`projections/discovery.rs:1-30`, `:704` `is_journal_name`).
  So `recognition.journal` would be offered as a projection source. It's
  excluded (D-guards).
- **Insights already benefits.** Plan 37's customer metrics hide behind
  `billing-clearing` when `customer:` sits only on invoices
  (`saas.rs:698` `ClearingRule`). Posted entries carry `customer:` on the
  revenue line, so those metrics turn on. `saas.rs:927` `recurring_horizon`
  treats the months after the last recognition as open, so unposted months
  read as "not closed", not as churn.
- **Optional dates** use `components/DateInput.svelte` (WebKit empty-date
  issue).
- **Perf.** `docs/development/perf-baseline.md`: `parse/parse_journal` takes
  253 ms at 200k transactions. Benches live in `crates/ledgeline-core/benches/`
  (`engine.rs`, `matching.rs`, `corpus.rs`).

## Design

### D-vocab. Tags

| Tag              | Where                                                                    | Value                                     | Meaning                                                                                                |
|------------------|--------------------------------------------------------------------------|-------------------------------------------|--------------------------------------------------------------------------------------------------------|
| `recognition`    | Source deferred/prepaid **posting**; every rule header; every posted entry's header | 6-char base-36 id, or `none`              | The schedule link. `none` on a source posting means "don't schedule this, don't ask again".            |
| `service-start`  | Source deferred/prepaid posting                                          | `YYYY-MM-DD`                              | First day of service. Defaults to the transaction date.                                                 |
| `term-months`    | Source deferred/prepaid posting                                          | integer ≥ 1                               | Length of service. Patrick's tag, kept.                                                                 |
| `service-end`    | Source deferred/prepaid posting, **only** when the term isn't whole months | `YYYY-MM-DD`, inclusive                   | Replaces `term-months` for odd terms (and for an early end, see D-credit).                              |
| `proration`      | Source deferred/prepaid posting                                          | `calendar`                                | Calendar-month periods with day-prorated stubs (D-anchor). Absent means anniversary periods.                  |
| `contract`       | Source deferred/prepaid posting                                          | `new` \| `renewal`                        | Inferred from the customer's history and editable (D-renewal).                                         |
| `customer` / `vendor` | Source postings, rule postings, posted entries' postings            | name                                      | Plan 37's attribution, unchanged. Revenue lines carry `customer`, expense lines `vendor`.              |
| `invoice`        | Invoice/bill **and** payment transaction headers                         | the document number                       | The strongest match signal (D-settle).                                                                  |
| `renewal-date`   | Main rule header                                                         | `YYYY-MM-DD`                              | service-start + term. Patrick's tag, kept on the rule so `print --forecast` shows it.                  |
| `recognized`     | Posted entry's header                                                    | the **scheduled** occurrence date         | Per-period identity. Duplicates are detected on `(recognition, recognized)`, never on the transaction date. |
| `restated`       | Catch-up adjustment's header                                             | the date of the restatement               | An adjustment under an id that fills no period (D4).                                                    |
| `match`          | AR/AP postings of the invoice and of the payment                          | plan 35 id                                | Settlement link (D-settle).                                                                             |

**Normalized from Patrick's examples.** He put `start-date` on the AR
posting and `term-months` on the deferred posting. Every recognition fact now
goes on the **deferred/prepaid posting being recognized**, because that's the
line a schedule spreads, and one transaction can hold several such lines with
different terms. `start-date` becomes `service-start`, a name that can't be
confused with hledger's special `date:` tag.

**The id.** It's generated like `matching::new_match_id` (`matching.rs:1606`),
from its own namespace and unique across `recognition` values. The generator
is factored so both modules share it.

**Why `recognized` holds the scheduled date.** A user can move a posted
entry's date (say Dec 1 → Nov 30), and the period it fills mustn't change.
`tag:recognized=2026-12` queries all December occurrences, because hledger
tag values are regexes.

### D-sched. Schedules: periods, dates and amounts

A **schedule** is one or more deferred/prepaid lines from one source
transaction that share a start, an end and a proration. It has one
`recognition` id, one rule set and one posted entry per period.

- **Several lines with the same terms** (say a subscription line plus a
  support line, both 12 months from Nov 17) form one schedule. They share an
  id, and each period's rule and entry hold a pair of postings per line. Each
  line can recognize into its own revenue or expense account. That keeps
  "one payment, one recurring entry".
- **Lines with different terms** (a 3-month onboarding package beside a
  12-month subscription) each get their own id and rules, so each period of
  each gets its own entry. The editor shows them as **Schedule A / B** under
  the one transaction. `(id, recognized)` stays unique without a third key.
- **Lines that aren't deferred are never touched.** Fees, one-time items and
  the bank leg are left alone.

**Periods.**

| Proration               | Periods                                                                                    | Entry dated                                                                 |
|-------------------------|--------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------|
| Anniversary (default)   | `[S + (k−1) months, S + k months)`, k = 1…n, with months added from S and clamped           | `S + (k−1) months` (**in advance**, Q2 answered: start Nov 17 → first entry Nov 17, last Oct 17; `renewal-date` stays `S + n months`) |
| Calendar (`proration: calendar`) | Each calendar month the service touches. The first and last may be partial              | The 1st of that month, or the service start date for an opening stub (Q12: the day a monthly payer would pay) |

Recognition is always monthly (D3). A quarterly or annual **billing** term
only changes `term-months`.

**Amounts** (exact `Dec`, rounded to the commodity's display precision, half
away from zero, like `Dec::div_int`):

- Anniversary: `a = round(T / n)`. Periods 1…n−1 get `a`, and period n gets
  `T − (n−1)·a`.
- Calendar: `f_first` and `f_last` are the day fractions of the partial
  months (days served ÷ days in that month), and `N` is the number of full
  months. Then `m = round(T / (f_first + N + f_last))`. Each stub gets
  `round(m · f)`, each full month gets `m`, and the **final** period gets the
  remainder `T − Σ others` (D3).

**Rules written.** Only as many as needed, under one id:

| Rule        | When                                                    | Form                                                                                                         |
|-------------|---------------------------------------------------------|--------------------------------------------------------------------------------------------------------------|
| First stub  | Calendar, first month partial                           | `~ 2026-11-17  …` (the service start date)                                                                   |
| Main        | Two or more equal periods                               | Calendar, or anniversary day ≤ 28: `~ monthly from FIRST to LASTDAY+1`. Anniversary day 29–31: `~ every Nth day of month from FIRST-OF-MONTH to LASTDAY+1` (fact 2) |
| Final       | The last period's amount differs from the main amount   | `~ 2027-10-17  …`, with the main rule's `to` set to this date                                              |

Worked examples (each one is a fixture case):

```journal
; Patrick's scenario 2, fixed and in advance: 12 × $1000, Nov 17 … Oct 17.
~ monthly from 2026-11-17 to 2027-10-18  Customer X Recognized  ; recognition: k7f3a2, renewal-date: 2027-11-17
    liabilities:deferred-revenue    $1000.00  ; customer: Customer X
    revenue:sales                  $-1000.00  ; customer: Customer X

; $10,000 over 12: 11 × $833.33 and a $833.37 true-up.
~ monthly from 2026-11-17 to 2027-10-17  Customer Y Recognized  ; recognition: p2m9qd, renewal-date: 2027-11-17
    liabilities:deferred-revenue     $833.33  ; customer: Customer Y
    revenue:sales                   $-833.33  ; customer: Customer Y
~ 2027-10-17  Customer Y Recognized  ; recognition: p2m9qd
    liabilities:deferred-revenue     $833.37  ; customer: Customer Y
    revenue:sales                   $-833.37  ; customer: Customer Y
```

**Contracts that end early, or are credited back** (see D-credit), get a new
`service-end` and are restated as in D4.

### D-anchor. Anchored vs unanchored billing (D3)

- **Unanchored** (the period runs from the signup date): anniversary periods,
  as above. Nothing more is needed.
- **Anchored to the 1st** (Stripe's `billing_cycle_anchor`): set
  `proration: calendar`. The service months are then calendar months, and
  each entry is dated the 1st, the day a monthly payer on that plan would
  pay. An opening stub is dated its service start. This is Q12's single
  rule: **an entry is dated the day its service period begins**, which is
  the anniversary for unanchored contracts and the 1st for anchored ones.
  Month-end close still works: at close, **Post through ‹month end›**
  writes every entry dated in the month, so the entries are *posted* at
  close but *dated* when the period began. Two common shapes, both handled
  without special cases:
  1. **One payment covers the stub and the year.** For example $12,000 for
     Nov 17 2026 to Nov 16 2027:
     - Nov 17–30 2026: 14/30 of a month, $466.67, dated 2026-11-17.
     - Dec 2026 to Oct 2027: 11 × $1,000, dated the 1st.
     - Nov 1–16 2027: the $533.33 remainder (16/30), dated 2027-11-01.
     - Rules: `~ 2026-11-17`, `~ monthly from 2026-12-01 to 2027-11-01`,
       `~ 2027-11-01`.
  2. **A separate stub invoice** ($466.67 for Nov 17–30), then an annual
     invoice on Dec 1. That's two schedules: one with a single `~ 2026-11-17`
     rule, and one for 12 full calendar months from Dec 1. Its rule is `~
     monthly from 2026-12-01 to 2027-12-01`, and with whole months there's no
     true-up.
- **Credit-back** (an upgrade or cancellation mid-term, D-credit). The
  provider credits the unused part of the old plan on the next invoice:
  - The credit line **debits** deferred revenue for that customer.
  - The editor sees a deferred debit against a customer with an active
    schedule and offers **End contract ‹id› on ‹date›**.
  - That sets `service-end`, cuts the rule's `to` at that date, adds a
    prorated final rule, and shows whether what's left of the schedule equals
    the credit.
  - A difference (the provider prorates by seconds, say) is offered as a
    final adjustment in the month of the change.
  - The new plan's line on the same invoice is an ordinary new schedule.
    Its `contract` is `renewal` when the customer has a previous contract.

### D-gate. When the helpers appear

An account's **accrual role** is one of `deferred-revenue`, `prepaid`,
`receivable` or `payable`. It's resolved in this order:

1. The nearest declared `kpi:` on the account or an ancestor
   (`kpi_resolver`, with `prepaid` added to `KpiRole`; expected type Asset).
2. Otherwise, with no `kpi:` anywhere in the subtree, the account's last
   segment, matched ASCII case-insensitively against its effective type:
   - Asset containing `receivable` → receivable (plan 37's rule).
   - Liability containing `payable` → payable (plan 37's rule).
   - Asset containing `prepaid` → prepaid. **New, used by the gate only.**
   - Liability containing `unearned`, or `deferred` together with `revenue`
     or `income` → deferred-revenue (Q4).
     **New, used by the gate only.**

   Insights keeps plan 37's narrower rule.

The ledger is **accrual** when any account resolves to one of these roles,
unless `ledgeline.toml` says `[books] basis = "cash"`. `basis = "accrual"`
forces the helpers on before any account is declared, which a new ledger
needs. With the gate off, every accrual route answers `{enabled: false}`, and
the UI shows nothing: no button, no editor section, no grouping and no chip.

### D1 made concrete: the recognition file

- **Location.** `recognition.journal` beside the main journal. A
  hand-written `[books] recognition-file = "books/recognition.journal"` in
  `ledgeline.toml` overrides it. The path is confined to the journal's
  directory, the same way `syncSourceDir` is (`ui_prefs.rs:236`), and is
  never read or written through a symlink.
- **Creation is explicit.** The first time a schedule is saved, the editor
  asks: "Create recognition.journal beside books.journal? It holds the
  schedules. Your books don't include it, so they stay a plain hledger
  journal." It's written through the vault's exclusive-create primitive,
  following `create_budget_file` (`budget_api.rs:1306`) but **without** the
  `include` line.
- **Contents.** A header comment (what the file is, the `-f` command that
  reads it, "do not include this file"), followed by the `decimal-mark` and
  `commodity` directives copied from the main journal for each commodity
  used. That way the file reads identically on its own (fact 8). After that
  come the schedules, one block group per id, in creation order, each group
  preceded by a comment line naming the customer or vendor and the source
  transaction: `; ── Customer X · k7f3a2 · 2026-11-17 Customer X Payment`.
- **Loading.** On each snapshot build, if the file exists, it's parsed with
  `parse_journal_with_styles` (inheriting the main journal's styles) into a
  `RecognitionBook { rules: Vec<PeriodicTransaction>, revision }`. **It is
  never merged into `Journal::periodic_transactions`.** The parse is cached
  by file identity and revision, so a main-journal edit doesn't re-read it.
  `watched_files` gains the path, so an outside edit refreshes.
- **What's in the file is Ledgeline's to write.** The file is edited through
  a block model in the style of `periodic.rs` ("an edit rewrites bytes only
  inside the spans it names"): append a group, replace a group, delete a
  group. Each edit is reparsed and verified: the target group reads back as
  the intended rules, and every other byte is unchanged. Rules the user wrote
  by hand are read the same way, and are shown and posted like any other.
  Ledgeline only refuses to *rewrite* a group it can't map back to a schedule
  (a block without `recognition:`, or with an `Unsupported` period); it says
  why and leaves it alone.

### D-guards. Keeping D2 true

- **Structural.** Rules are never in `Journal::periodic_transactions`, so the
  budget report, budget gaps, budget editor and projection seed can't see
  them.
- **The file is included by mistake.** If the recognition file appears in
  `journal.source_files`:
  - Problems shows the warning `recognition-file-included` ("hledger and the
    Budget tab will treat every schedule as a budget goal").
  - Ledgeline doesn't load it a second time.
  - Defensively, every `periodic_transactions` reader (budget, gaps, seed)
    filters out rules whose header has a `recognition:` tag. A `recognition`
    rule is never a goal anywhere in Ledgeline.
- **Projection discovery** leaves out the recognition file's path, so it is
  never offered as a scenario source. The projections docs explain copying it
  into a `projection-*.journal` (and dropping its `to` bounds) for a revenue
  run-rate forecast.
- **Budget file collision.** A `recognition-file` that names `budget.journal`,
  the main journal, or any included file is refused, and Problems names it.

### D6 made concrete: due entries, catch-up and auto-post

An occurrence is **due** when all of these hold:

- its date is on or before **today**: the server's local date, which the
  client's `today` can lower and never raise;
- `(id, date)` isn't in the posted set;
- its date is after the last restated cut for that id (D4).

A due occurrence on or before `closed-through` is **due in a closed period**.
It's listed separately, unchecked, and can only be posted as a catch-up
adjustment dated today, never into the closed month.

**The Recognition overlay**, opened from a toolbar button beside Find
Matches:

- The button shows a badge with the due count, and has a ▾ dropdown like
  Find Matches (amendment A1) with the lookback for "Needs a schedule".
- There are three tabs:

| Tab                 | Lists                                                                                                                         | Primary action                     |
|---------------------|-------------------------------------------------------------------------------------------------------------------------------|------------------------------------|
| **Due**             | The catch-up table: due occurrences grouped by month, then by customer/vendor, every row checked                                 | **Post through ‹date›**            |
| **Needs a schedule** | Deferred-revenue credits and prepaid debits with no `recognition:` tag, inside the lookback, written in Ledgeline or by an external app | Open the editor's Accrual section |
| **Invoices**        | Settlement proposals (D-settle), plus open invoices and bills with no payment yet, and payments with no invoice                  | Link / Link all clear              |

**The catch-up table** (D6):

```
Recognition · Due                                            Post through Nov 17 ▸
──────────────────────────────────────────────────────────────────────────────────
▾ ☑ Oct 2026 · 41 entries · $44,180.00
     liabilities:deferred-revenue  $512,400.00 → $468,220.00   ties ✓
     ▸ ☑ Customer X   Oct 17   deferred-revenue → revenue:sales        $1,000.00
     ▸ ☑ Customer Y   Oct 17   deferred-revenue → revenue:sales          $833.33
     …
▸ ☑ Nov 2026 · 37 entries · $41,250.00                                 ties ✓
  ☐ Closed periods (before Oct 1) · 2 entries · $1,666.66   post as one adjustment today
```

- **Ties.** For each month and each deferred/prepaid account, it shows the
  account's balance at month end before and after posting, and the schedules'
  **scheduled remaining balance** at that date (Σ of every schedule's
  unrecognized amount). "ties ✓" when they agree. Otherwise it shows the
  difference, linked to **Needs a schedule**, where an unscheduled deferral
  is usually the cause.
- **Row expand.** Edits the date (never past today, never into a closed
  period), the amount, the accounts and the description.
  - Changing an amount asks: **absorb the difference in the final period**
    (rewrites the final rule; the default), or **leave the schedule** (a
    `recognition-sum` warning stays until it's fixed).
  - The posted entry keeps `recognized:` set to its *scheduled* date.
- **Post through ‹date›.** The date is the end of the latest checked month,
  capped at today.
  - All checked rows go to `add_transactions` in **one** call: one reparse,
    date-ordered placement, so a per-year included file receives its own
    rows.
  - Under the write lock, the server rebuilds the posted set from the current
    journal and drops anything already posted. A double click or a second
    window can't duplicate.
  - It refuses any date after today.
- **Entry shape.** A posted entry is the rule as `hledger print --forecast`
  would print it, without `generated-transaction`, and with `recognized:`
  added:

  ```journal
  2026-11-17 Customer X Recognized  ; recognition: k7f3a2, renewal-date: 2027-11-17, recognized: 2026-11-17
      liabilities:deferred-revenue    $1000.00  ; customer: Customer X
      revenue:sales                  $-1000.00  ; customer: Customer X
  ```

  It has no status mark and no `reviewed:` tag (see **(Q6)**).

**Auto-post** is a UI preference, `autoRecognize` (bool, default `false`,
next to `autoSync` in `ui_prefs.rs:253` `SPECS`). When it's on, the server
posts the due entries:

- on load,
- once a day after local midnight, from the same scheduler shape as bank
  sync, and
- after any write that adds a schedule.

Due entries in closed periods are never auto-posted. A toast says what was
written ("Posted 37 recognition entries through Nov 17"), with a link to the
month's group row.

### D4 made concrete: restatement

A **restatement** happens when a saved edit, made in Ledgeline, changes a
schedule's inputs: the deferred line's amount, its account, `service-start`,
`term-months`/`service-end`, `proration`, the revenue/expense target, or the
customer. Saving the source transaction then opens a **preview** before
anything is written:

```
Restate Customer X · k7f3a2                     $12,000.00 → $11,400.00
─────────────────────────────────────────────────────────────────────────
Rule (recognition.journal)
  − ~ monthly from 2026-11-17 to 2027-10-18 …  $1000.00
  + ~ monthly from 2026-11-17 to 2027-10-18 …   $950.00
Posted entries                           before      after     action
  2026-11-17  (closed)                  1,000.00   1,000.00   kept
  2026-12-17  (closed)                  1,000.00   1,000.00   kept
  2027-01-17                            1,000.00     950.00   rewrite
  2027-02-17                            1,000.00     950.00   rewrite
  2027-03-17                            1,000.00     950.00   rewrite
Catch-up adjustment  2027-03-31  restated: 2027-03-20       −100.00
Schedule total after: 11,400.00  =  deferred line 11,400.00  ✓
                                                  [Cancel]  [Apply all]
```

- Entries after `closed-through` are rewritten in place. That's one
  fingerprint-checked batch through `apply_batch`, changing amounts, and the
  date and `recognized:` when the period moved.
- Entries on or before it are kept, and their total difference becomes **one**
  adjustment, dated the last day of the current month and no later than
  today. It's tagged `recognition: ID, restated: TODAY` and counts toward the
  schedule's total but fills no period.
- An occurrence the new schedule drops (a shortened term) is deleted if it's
  open, or folded into the adjustment if it's closed.
- Write order: first the source transaction (the user's save), then the
  rewritten entries and the adjustment in one batch, and last the rule group.
  Every step checks the revision it read. If a later step fails, the earlier
  ones stand, and the mismatch shows up as a `recognition-sum` finding with
  **Review restatement** as its fix. The same preview then finishes the job.
- **Edits made outside Ledgeline** aren't restated automatically (the ask
  puts them on the user). The same mismatch is detected on refresh and
  surfaced as that finding.

### D-credit. Ending a contract early

**End contract on ‹date›** (from the editor, or offered on a credit line)
restates with `service-end = date`. The preview additionally shows what's
left of the schedule and how it compares with the credit or refund line. Any
difference becomes the final adjustment (see D-anchor, credit-back).

### D-settle. Invoices, bills and payments (D5)

- **Candidates.** These use different rules from plan 35's transfer
  candidates; only the tag, ids and link machinery are shared:
  - **Invoice side:** an AR debit (or AP credit), written by hand or by an
    external app, with no `match` tag.
  - **Payment side:** an AR credit (or AP debit) in a transaction with a cash
    leg, with no `match` tag.

  Real, written postings with one amount and no cost, as in plan 35 D3.
- **Pairing** (1:1). Same receivable/payable account, same commodity, and the
  payment dated on or after the invoice and within the lookback (default 180
  days). The ranking is:
  1. The same `invoice:` value on both, or the payment's description
     containing the invoice number as a whole word.
  2. The same customer/vendor. The payment's customer is its `customer:` tag,
     or else the customer most often seen with the payment's payee similarity
     key (`review.rs:204` `similarity_key`) in earlier matched payments.
  3. The amount: exact first, then `0 < invoice − payment ≤ tolerance`. The
     tolerance defaults to 3% of the invoice, with no cap (Q3 answered).
  4. The smallest date gap.

  Ties and ambiguity are handled as in plan 35 D6. Only unambiguous pairs
  with an invoice-number or customer signal and an exact amount are
  **clear**.
- **Fees.** When the payment is short, the card proposes the fee edit to the
  payment transaction:
  - The AR posting is set to the invoice amount, made explicit even if it was
    elided.
  - A fee posting for the difference is added. Its account is the one most
    recently used for fees in matched payments on that AR account; with no
    history the field is empty and required.

  The two AR legs then cancel exactly, so the link is a valid plan 35 link.
  An overpayment is **check** only, and the card says so; customer credits
  are later.
- **Link.** It's one batch: the fee edit (if any) and `match:ID` on both AR
  postings. Unlink, reject (`nomatch`) and forget are plan 35's routes,
  unchanged.
- **Engine changes in `matching.rs`:**
  - A posting is a **settlement posting** when its account resolves to
    receivable or payable (D-gate).
  - `LinkIndex` accepts a link whose members are all clearing postings on
    one account, **or** all settlement postings on one account.
    `NotClearing` keeps its meaning for anything else.
  - `Transfers::of` skips settlement links: an invoice and its payment stay
    two rows. Each row shows a "Paid" chip (invoice) or "Pays INV 1042"
    (payment), and clicking it jumps to the partner.
  - The in-transit finding stays clearing-only. An unpaid invoice isn't a
    problem; it's an open invoice (aging comes later).
- **After an import or sync** that added payments to AR or AP, the result
  screen offers **Match invoices**, as plan 35 does for transfers.

### D-editor. The Accrual section in the transaction card

A collapsible section in `ReviewOverlay`, built like `SimilarSection`, for
both add and edit (single edit only, not queue cards, as with Find match).
What it shows depends on the card's postings. It's recomputed on every edit,
client-side, from the posting accounts' roles (served once with
`/api/accrual`).

| The card has                                                                 | The section shows                                                                                         | Opens   |
|------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------|---------|
| A deferred-revenue credit or prepaid debit with no `recognition:`            | **Recognize over time** form, one per distinct terms (Schedule A, B …)                                     | open    |
| The same, already scheduled                                                  | Schedule summary: "12 × $1,000 · 4 posted · 1 due · next Dec 17", with **Edit schedule** and **End contract** | closed  |
| A revenue credit or expense debit beside a cash, AR or AP leg, and no deferral | "Earned **at once** ◉ / **over a term** ○". Over a term converts that line to the suggested deferred/prepaid account and opens the form | closed  |
| An AR debit / AP credit (an invoice or bill)                                 | Invoice number, customer/vendor, and payment status ("Unpaid" or "Paid Dec 3 · $970 + $30 fee")           | closed  |
| An AR credit / AP debit from cash (a payment)                                | Invoice candidates, as in Find match's focused view                                                       | open if a clear candidate exists |
| A deferred debit outside a recognition entry (a credit note)                 | "Credits ‹customer›'s contract ‹id›?" → End contract (D-credit)                                            | open    |

**The form** has these fields:

- **Customer/vendor**: tag autocomplete from existing `customer:`/`vendor:`
  values. Optional (the ask).
- **Service start**: `DateInput`, defaulting to the transaction date.
- **Term**: chips for 1, 3, 6, 12, 24 and 36 months, or an end date.
- **Recognize into**: an account with autocomplete. The default is the
  previous contract's target, else the account most often credited from this
  deferred account in posted entries, else the most-used revenue (or expense)
  account for this payee.
- **Timing**: "On the start date's anniversary" or "Calendar months
  (anchored, prorated)".
- **Contract**: new or renewal, with the inferred value marked.

Below the fields is a live **preview**: the period table (dates and amounts,
with the stub and true-up rows labelled), the first and last dates, and the
rule text exactly as it will be written. **Save** writes, in order:

1. The source transaction with its tags, through the editor's normal save.
2. The rule group in the recognition file, after asking to create the file
   the first time.
3. If `autoRecognize` is on, the entries that are already due.

**Suggesting a deferred account.** The suggestion is the deferred-revenue
(or prepaid) account this payee or customer used before. Failing that, it's
the only account with that role, or a list when there are several. With
none, the form offers to declare `liabilities:deferred-revenue ; type: L,
kpi: deferred-revenue` (or `assets:prepaid ; type: A, kpi: prepaid`) through
the Accounts editor's existing write path.

### D-renewal. Prefill from the previous contract

When the form has a customer (or vendor), the engine finds that party's
**latest schedule** (by service start) and prefills its term, target
accounts, timing and line structure. A banner says "Prefilled from Customer X
· Nov 2025 · annual · k3d8w1", and every field stays editable (annual →
quarterly, say). `contract` is inferred:

- `renewal` when the party has a previous schedule, or any earlier posting
  carrying their `customer:`/`vendor:` tag on a revenue/expense or
  deferred/prepaid account;
- `new` otherwise.

An unattributed source (no tag) gets a suggested customer from the payee key
→ customer map built from tagged history. It's offered, never applied
silently.

### D8 made concrete: the journal list

- **Grouping in the engine.** Grouping happens in the row model, beside
  `Transfers` (`journal_api.rs:90`), as a per-snapshot
  `OnceLock<Arc<RecognitionRows>>`.
  - In each calendar month, the filtered posted entries (any transaction with
    both `recognition` and `recognized`) collapse into one **group row**
    anchored at the latest-dated member, when there are at least two.
  - `total`, offsets, positions and the summary count group rows as rows, as
    plan 35 D11 does.
  - The row carries `recognitionGroup: {month, count, totals: {commodity →
    Dec}, expanded}`. The label is "Recognition · Nov 2026 · 37 entries ·
    $41,250.00".
- **Expand in place.** `?expand=2026-11,2026-10` on `/api/journal/rows`
  serves the members as ordinary rows right under the group row, and the
  param is part of the filter cache key. Keyboard: `l` toggles it, as on a
  transfer row.
- **The chip.** A filter chip **Recognition: Grouped / Hidden / Shown**
  (`?recognition=group|hide|show`, remembered as the `recognitionRows` UI
  preference, default `group`).
  - **Hidden** removes the rows from `rows` only. **Totals, charts and running
    balances still include them** (they are postings, and D8 says they
    count), and the footer says "incl. 412 hidden recognition entries".
  - With a pinned running balance (plan 36), entries are never grouped: every
    row needs its own balance.
- **Search** still finds members: a group row's haystack is its members'
  haystacks, and a hit expands that group.

### Other readers

| Reader                                 | Change                                                                                                           |
|----------------------------------------|------------------------------------------------------------------------------------------------------------------|
| Subscriptions (`subscriptions.rs`)     | Skip posted recognition entries, so prepaid amortization isn't a "subscription".                                 |
| Review queues (`review.rs:486`)        | Recents and Unknowns skip posted recognition entries **(Q6)**.                                                   |
| Insights Top Transactions (`insights.rs:873`) | Unchanged in v1: a recognition entry is revenue. Revisit if 37 rows a month crowd it out **(Q7)**.          |
| Insights startup metrics               | No change. The entries' `customer:` tags turn on per-customer MRR. Later phase below.                             |
| Projections                            | Posted entries are history like any other transaction. The rules never enter (D-guards).                          |
| Problems (`view/checks.rs:367`)        | New findings below.                                                                                              |

**Problems findings:**

| Rule                         | Severity | When                                                                                                         | Fix offered            |
|------------------------------|----------|--------------------------------------------------------------------------------------------------------------|------------------------|
| `recognition-file-included`  | warning  | The recognition file is in `journal.source_files`                                                             | Docs link              |
| `recognition-duplicate`      | warning  | Two transactions carry the same `(recognition, recognized)`                                                   | Delete one             |
| `recognition-sum`            | warning  | A schedule's rules plus its adjustments ≠ its source line(s), or a posted entry's amount ≠ its rule's          | Review restatement     |
| `recognition-orphan`         | info     | A posted entry or source posting names an id with no rule, or a rule's id has no source posting                 | Open in editor         |
| `recognition-closed-due`     | warning  | A due occurrence on or before `closed-through` is unposted                                                     | Post as adjustment     |
| `recognition-rule-account`   | warning  | A rule posts to an account that isn't declared (strict hledger wouldn't catch it without `--forecast`, fact 8) | Open the rule          |

### D-settings. Where things are stored

| Setting                         | Where                                                                                     | Written by                                                                    |
|---------------------------------|-------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------|
| `basis = "cash" \| "accrual"`   | `ledgeline.toml` `[books]`                                                                | Hand only                                                                     |
| `recognition-file`              | `ledgeline.toml` `[books]`                                                                | Hand only                                                                     |
| `closed-through = "YYYY-MM-DD"` | `ledgeline.toml` `[books]` (Q1 answered)                                                  | The explicit **Close books through…** action (creates the file if missing, like `[sync]`), or by hand |
| `autoRecognize`                 | UI prefs (`ui-prefs.toml`; a ledger can override it under `[ui]`)                         | Settings                                                                      |
| `recognitionRows`               | UI prefs                                                                                  | The chip                                                                      |
| Settlement lookback / tolerance | UI prefs, beside the match settings                                                       | The ▾ dropdown                                                                |

`[books]` values are validated like `[ui]`: dates are parsed, `basis` is a
closed set, and the path is confined. None of them runs anything.
`ui_prefs.rs`'s module doc and `docs/settings/settings-files.md` gain the
third exception.

### D-lib. Library first: the invoicing app uses the same functions

Patrick, 2026-11-17:

> We are going to be creating a separate invoicing app and it is going to
> load ledgeline crates as libraries. That invoicing app will need to be able
> to write entries for a journal and for the revenue recognition periodic
> stuff, but rather than write itself, I want to be sure it can use the same
> logic and everything else that ledgeline uses.

So every operation in this plan that reads or writes the books is a public
function in `ledgeline-core`. The server only wraps it with HTTP, the
snapshot, and UI preferences.

- **`accrual::ops`** holds the writes. Each takes the main journal path (or
  a loaded editor), `today`, and the expected fingerprints, and returns typed
  results and errors:
  - create `recognition.journal`;
  - save schedules (D-editor: the source tags first, then the rule group);
  - post due entries (D6: rebuilds the index, drops anything already posted,
    refuses dates after today);
  - apply a restatement (D4);
  - link a settlement, with its fee edit (D-settle);
  - read and write `ledgeline.toml [books]` (`basis`, `closed-through`).

  They all go through `edit.rs` (`add_transactions`, `apply_batch`, and
  `save`'s `ExternalChange` guard), so an invoicing app and a running
  Ledgeline can write the same books safely: the second writer is refused
  and retries, and nothing is clobbered. The shared part of the server's
  `ledgeline.toml` editor (`ui_prefs.rs` `edit_ledger_text`) moves to core.
- **Builders.** Invoice, bill and prepayment builders take a typed spec
  (customer/vendor, invoice number, lines, deferred lines with
  `service-start`/`term-months`/`proration`, `contract:`). They return the
  `Transaction` plus a `ScheduleSpec` for each deferred line, so one `ops`
  call writes an invoice and its schedule. The tag-name constants and the
  recognition-file path resolution are public.
- **No hidden state.** No globals, no server types, and `today` is always a
  parameter. `autoRecognize` stays a server preference; core only offers
  `post_due`.
- **Enforced by a test.** `tests/accrual_library.rs` uses only public
  `ledgeline_core::` paths to run the whole flow an invoicing app would.
  It also covers an external write between load and save, which must give
  `ExternalChange` and write nothing.
- **Semver-visible.** Every public item in `accrual` has rustdoc, and the
  module docs include an example. The API is kept small.

## Performance

The target is a 200k-transaction ledger with 5,000 active schedules (consumer
scale), 36 months of history, and about 150k posted entries.

| Work                                | How                                                                                                          | Cost                                  | Target (200k)  |
|-------------------------------------|--------------------------------------------------------------------------------------------------------------|---------------------------------------|----------------|
| `RecognitionIndex` build            | One pass over transactions and their postings' own tags. It collects source postings by id, posted entries as `(id, recognized) → position`, and adjustments. Ids are interned to `u32` and dates are `i32` day numbers | O(T + P)                              | `index_with_roles` < 15 ms (13.4 ms measured) |
| Recognition file parse              | `parse_journal_with_styles`, cached by file identity and revision                                            | O(file); skipped when unchanged       | < 15 ms (5k groups) |
| Due set                             | For each rule, `occurrences(first, min(today, to))`, then a hash lookup in the posted set                     | O(Σ periods ≤ today)                  | < 5 ms         |
| Ties                                | Month-end balances of the deferred/prepaid accounts from one sorted pass                                     | O(P_accrual)                          | < 5 ms         |
| Group rows                          | Per filter result, one scan of `positions`, bucketing members by month                                       | O(rows)                               | < 2 ms         |
| Post 5,000 entries                  | One `add_transactions` call: format, splice, **one** reparse                                                  | ≈ one parse (253 ms) + formatting     | < 600 ms       |

- **The index target is 15 ms, not 10 (Patrick, 2026-11-18).** Just
  touching every posting of a 200k ledger costs about 8 ms, so the scan
  (parallel across ≤8 threads) plus its serial merge lands at 13.4 ms;
  getting under 10 would need incremental indexing, which isn't worth it.
- **Per snapshot, lazily.** Like `transfers` and `review`, the index is built
  on first use, in a `OnceLock`. A refresh builds a new snapshot, and a
  rebuild costs one pass. No incremental bookkeeping is needed beyond the
  revision-keyed parse cache for the recognition file. The `/api/accrual`
  counts come from the same index.
- **Bench.** Add `benches/accrual.rs`, and extend `corpus.rs` to emit N
  contracts (deferred source lines with tags, a matching recognition file,
  and about half the periods posted). Benches: `accrual/index`,
  `accrual/due`, `accrual/schedule_build_5k`, `accrual/group_rows`,
  `accrual/post_5k`. Record the figures in
  `docs/development/perf-baseline.md`.

## Scope

In:

- Everything above for revenue and expense.
- Settlement matching 1:1 with fees.
- Restatement and early end.
- The recognition file.
- Grouping and hiding in the journal.
- Problems findings.
- Docs.

Out (later phases or **(Q)**):

- N:M settlement (one payment for several invoices, partial payments) and AR/AP
  aging.
- Customer credits from overpayment.
- Multi-currency schedules with FX revaluation. v1 recognizes in the source
  line's commodity at cost; a line with `@` cost is refused, with a reason.
- Insights follow-ups (renewals by month, new vs renewal, billed/paid).
- Adopting historical manual JEs. v1 offers **Schedule the remaining
  balance** (start = next period, T = the line's unrecognized balance)
  instead of rewriting history.

## Phase 1 — Engine (core, PR 1)

New module `crates/ledgeline-core/src/accrual/`:

- `mod.rs`: `AccrualRole`, `accrual_resolver` (D-gate), `is_accrual_ledger`,
  and the tag-name constants.
- `schedule.rs` (pure):
  - `ScheduleSpec { lines, start, end, proration, description, header_tags }`
    → `Schedule { periods: Vec<Period { date, amounts }>, rules: Vec<RuleText> }`.
  - Rule text is rendered in the main journal's styles through
    `JournalStyles`.
  - `periods_of(rules)` reads them back with `budget::occurrences`. A test
    asserts that it equals `periods`.
- `book.rs`: `RecognitionBook` (parse, cached) and `RecognitionDoc` (block
  model: append, replace and delete a group, plus verify), on the
  `periodic.rs` discipline.
- `index.rs`: `RecognitionIndex::build(journal, book)`. It provides
  schedules by id, posted entries by `(id, date)`, adjustments, source
  postings, schedules by customer/vendor (for renewal), the payee → customer
  map, and the integrity issues behind the findings.
- `due.rs`: `due(index, book, today, closed_through)`, `ties(...)`, and
  `entries_to_post(...) -> Vec<Transaction>` (the D6 entry shape).
- `restate.rs`: `restatement(old, new, posted, closed_through, today) ->
  Restatement { rule_change, rewrites: Vec<(Tindex, Vec<EditOp>)>, deletes,
  adjustment: Option<Transaction> }`.
- `settle.rs`: candidates, `propose`, `fee_ops`, and `link_ops` (wrapping
  `matching::link_ops`).

Changes to existing code:

- `reports/kpi.rs`: `KpiRole::Prepaid` (`ALL` grows to 9) and its expected
  type (Asset). Insights ignores it in v1. Closed-tag lists: `wire.rs` `kpi`
  values, and `accounts/model.ts` `SPECIAL_TAGS` (Phase 3).
- `matching.rs`: settlement postings in `LinkIndex`; the shared id generator.
- `view/rows.rs`: `Transfers::of` skips settlement links; `RecognitionRows`.
- `view/checks.rs`: the six findings.
- `reports/subscriptions.rs`: skip posted recognition entries.
- `review.rs`: queue exclusion **(Q6)**.
- `projections/discovery.rs`: an exclusion list parameter. The server passes
  the recognition path.
- Budget/seed readers: a defensive `recognition:` rule filter.

Tests (unit, in-module, plus `tests/accrual_*.rs` on the fixture):

- **Schedule properties** (proptest over amounts from 0.01 to 10⁹, terms 1–120,
  any start date including the 29th–31st and leap days, both prorations,
  1–4 lines, precision 0–4):
  - Σ amounts = T exactly, per line.
  - Every full-month amount is equal, and only the final period differs.
  - Dates strictly increase, with at most one per calendar month.
  - The rules read back through `occurrences` give exactly `periods`.
  - The main rule's `to` is the day after its last occurrence, or the final
    rule's date.
  - Day 29–31 uses `every Nth day of month`.
- **Due and post:**
  - Nothing after today is ever due. A proptest varies `today` and `closed_through`.
  - Posting is **idempotent**: post, rebuild the index, and nothing is due
    for the posted dates.
  - Posting twice with a stale list writes nothing new.
  - An entry whose date the user moved still fills its period.
  - A duplicate `(id, recognized)` gives the finding.
- **Restatement:**
  - Closed periods are kept and one adjustment is written.
  - Open periods are rewritten.
  - A shortened term deletes or folds the dropped periods.
  - After applying, Σ (posted + adjustment + future) = the new total.
  - A no-op edit gives an empty restatement.
- **Settlement:**
  - An invoice-number match beats an amount-only match.
  - A fee within tolerance produces a balanced fee edit, and the AR legs
    cancel.
  - Beyond tolerance there's no proposal.
  - An overpayment is check.
  - Settlement links don't collapse into transfer rows and aren't broken.
  - Plan 35's tests still pass unchanged.
- **Gate:** declared roles; name inference only with no `kpi:` in the
  subtree; `basis` overrides both ways; a household fixture (`sample.journal`)
  is not accrual.
- **Guards:**
  - An included recognition file raises the finding.
  - The budget report and projection seed ignore `recognition:` rules even
    when they're included.
  - Discovery leaves out the file.
- **hledger parity** (opt-in, like plan 35's): for each fixture schedule,
  `hledger -f books.journal -f recognition.journal print --forecast=S..E
  tag:recognition=ID` gives the same dates and amounts as `periods`. `bal`
  over the posted entries equals the index's posted totals.
- **Bench:** see Performance.

## Phase 2 — Wire (server, PR 2)

Every route is a thin wrapper over a public `accrual` function in core (D-lib). No write logic lives in the server.

| Route                                         | Does                                                                                                            |
|-----------------------------------------------|-----------------------------------------------------------------------------------------------------------------|
| `GET /api/accrual`                            | `{enabled, basis: {value, source}, roles: {account → role}, file: {path, exists, included}, closedThrough?, counts: {due, dueClosed, needsSchedule, invoices}}` |
| `GET /api/recognition/due?today=`             | Months → groups → entries, with ties and the closed section                                                      |
| `POST /api/recognition/post`                  | `{generation, through, entries: [{id, recognized, edit?}], absorb?}` → writes as in D6; `409` on a stale generation |
| `GET /api/recognition/for/{index}`            | The schedules of one source transaction: periods with status (posted@position, due, future, closed), renewal prefill, suggestions |
| `POST /api/recognition/preview`               | An edited source transaction plus a spec → the schedule, the rule text, and the restatement if the id exists     |
| `POST /api/recognition/save`                  | `{index, fingerprint, schedules, restatement?}` → the writes in D-editor and D4                                 |
| `POST /api/recognition/file`                  | Create the file (D1). `409` when something is in the way                                                        |
| `POST /api/books/closed-through`              | Set or clear `closed-through` (explicit action)                                                                  |
| `GET /api/settlements?lookback=`              | `{generation, counts: {clear, check}, proposals, open: {invoices, payments}}`                                    |
| `GET /api/settlements/for/{index}`            | The focused candidates for one transaction                                                                       |
| `POST /api/settlements/link`, `/link-all`     | Link, optionally with the fee edit. Unlink/reject/forget reuse `/api/matches/*`                                 |
| `GET /api/journal/rows`                       | Gains `recognition=group\|hide\|show` and `expand=`. `Row.recognitionGroup`. The summary gains `recognitionHidden` |

- **Snapshot:**
  - Add `recognition: Option<Arc<RecognitionBook>>`.
  - `watched_files` gains the file.
  - The index is a lazy `OnceLock` in the view cache.
- **The `[books]` reader/writer** lives beside `feeds::config`, through
  `edit_ledger_text`.
- **The `autoRecognize` scheduler** follows the sync scheduler's shape. The
  e2e harness gets a fake clock through the existing `today` plumbing.
- **Tests:**
  - Endpoint tests on a temp copy of the fixture (`*_endpoints.rs`): create
    the file; save a schedule (the bytes of both files); preview; post due
    (bytes, placement in the per-year file, idempotency, refusal of future
    dates); restate (closed and open); settle with a fee; rows grouping,
    expand and hide (with totals unchanged); gate off on `sample.journal`.
  - Native goldens: `requests.tsv` gains `/api/accrual` on `sample.journal`,
    which pins `{enabled: false}`.
  - Run `just test-integration` after the shape changes (memory:
    live-integration-tests-skip).

## Phase 3 — UI (web, PR 3)

- `web/src/lib/accrual/`: types, decoders, the `model.ts` mirrors of the
  schedule preview (display only; the engine computes), and stores.
- **Journal toolbar:** the **Recognition** button with a badge and ▾
  (D6), shown only when `enabled`. Its key is `a` (Q8).
- **`RecognitionOverlay.svelte`**, reusing `ReviewOverlay`'s frame, keys and
  motion:
  - The **Due** table: grouping, the tri-state month checkboxes (from
    `SimilarSection`'s `checkState`), row expand, ties, and the closed
    section.
  - **Needs a schedule**.
  - **Invoices**, reusing the matcher card parts.
- **`AccrualSection.svelte`** in `ReviewOverlay` (D-editor), with
  `ScheduleForm.svelte`, `SchedulePreview.svelte`, `RestatementPreview.svelte`
  and `SettlementPicker.svelte`. All dates use `DateInput`.
- **Journal rows:** the group row, expand/collapse, the chip in `FilterBar`,
  the footer note, and the "Paid" / "Pays INV" chips.
- **Settings:** an `autoRecognize` toggle, **Close books through…** with a
  confirmation, and the settlement tolerance.
- **Accounts editor:** the `kpi: prepaid` option.
- **Vitest** with `fakeEngine`, covering:
  - which section shows for each card shape in the D-editor table;
  - the renewal banner;
  - the absorb prompt;
  - the chip's three states;
  - the group row keyboard.

### Phase 3b — Full transaction details on match cards (Patrick, 2026-11-18)

**Ask.** A settlement card shows one posting per side (e.g.
`accounts-receivable +$x` / `accounts-receivable −$x`). That is enough for
the headline, but sometimes the rest of each transaction is needed: every
account and amount, and every comment.

**Design.**
- **Shared body.** Extract the read-only body of `journal/TxnDetailSheet.svelte`
  into `journal/TxnDetailBody.svelte`. The body shows:
  - date, status and description;
  - every posting with its full account and amount, plus the posting's comment;
  - the transaction comment;
  - the `Tags:` line.

  The sheet keeps its modal and actions and renders the body, so its output
  does not change. The body takes `highlight?: number` (a posting index) and
  marks the matched posting so it stands out among the others.
- **Per card, inline.** Each side box on a settlement card has a "Details ▸/▾"
  disclosure, and either one opens (or closes) BOTH sides of the pair
  (Patrick, 2026-11-18: there are no per-side toggles, and no separate "Both").
  Each side renders `TxnDetailBody` in place under itself, with its matched
  posting highlighted. On wide layouts the two sides stay side by side, so the
  halves line up. The `d` key toggles the focused card's pair.
- **Panel-wide toggle.** A "Show details" toggle in the panel header expands
  every card. It is session state, not a pref, and resets on reload.
- **Open lists.** Rows in the "Open invoices" and "Open payments" lists, and
  in "Needs a schedule", have no partner, so each keeps its own single
  disclosure.
- **Open in editor.** Each expanded body has a link that opens the transaction
  in the review editor, as "Needs a schedule" already does.
- **Data.** Fetch lazily with the existing `GET /api/transactions/{index}` on
  first expand.
  - Cache by `(index, generation)`; a new generation drops the cache.
  - The list payload does not grow, so the 200k list stays at ~89 ms.
  - While the fetch is in flight, show a one-line skeleton.
  - If the fetch fails, say so inline with a Retry.
- **Scope.** Settlement cards, on both the Invoices tab and the `/settlements`
  page, plus the open lists. The plan 35 transfer-match overlay has the same
  one-line halves; reusing the body there is a follow-up, not part of this
  change.

**Tests (vitest, `fakeEngine`).**
- `TxnDetailBody`:
  - every posting, its comment and its amount;
  - the transaction comment and the tags line;
  - the highlighted posting.
- `TxnDetailSheet` still renders the same content (the existing tests pass
  unchanged).
- Settlements panel:
  - Details on either side opens the pair, each side fetched once, and
    expanding it again fetches nothing;
  - `d` toggles the focused card's pair;
  - "Show details" expands all;
  - a new generation refetches;
  - a failed fetch shows Retry.

## Phase 4 — Fixture, e2e and docs (PR 4)

**Fixture `fixtures/accrual/`.** Synthetic, invented names, and no real data.
Clock: 2026-11-17. Files:

- `books.journal`: main, `include`s `books-2025.journal`.
- `books-2025.journal`.
- `recognition.journal`.
- `README.md`: per-case expectations.
- `capture.sh` → `expected/`, hledger's own output, never regenerated from
  Rust.

Cases:

| Case                  | What it pins                                                                                   |
|-----------------------|------------------------------------------------------------------------------------------------|
| Point-in-time sale    | Scenario 1: no section opens except customer                                                   |
| Annual prepay         | Scenario 2, with posted entries Dec 2025 … Oct 2026 and Nov 2026 due                             |
| Multi-line payment    | A fee, a one-time item, and two deferred lines with the same terms → one schedule; a third line with 3-month terms → schedule B |
| Invoice → payment     | Scenario 3: recognition began before payment; the payment is $30 short (a fee proposal)         |
| Billed after delivery | Scenario 4: an invoice to revenue, matched to a payment with an exact amount                    |
| Renewal               | Customer X renews annual → quarterly-billed annual; prefill and `contract: renewal`             |
| Anchored              | A calendar schedule with stub and true-up (D-anchor, shape 1)                                         |
| Credit-back           | An upgrade with a credit line, ending the old contract                                          |
| Rounding              | $10,000 / 12                                                                                   |
| Prepaid D&O + E&O     | Two vendors, different start dates, `kpi: prepaid`, AP bill, separate schedules                 |
| External invoice      | AR vs deferred with `invoice:` only → Needs a schedule                                          |
| Closed period         | `ledgeline.toml` `[books] closed-through = "2026-09-30"`, one unposted September occurrence      |
| Duplicate             | A second hand copy of one posted entry → finding                                               |

**e2e** (`e2e/accrual.e2e.ts`, run by the lead, never by agents). It uses a
new engine on **port 5092**, serving a scratch copy in
`target/e2e-accrual/`, because it writes. The scenarios:

- The badge shows the due count.
- The Due table ties.
- Post through Nov 17 writes the entries, the badge goes to 0, and a second
  post writes nothing.
- The journal shows "Recognition · Nov 2026 · N entries", expands and hides,
  and the totals don't change.
- New schedule from the editor:
  - The file-creation prompt appears.
  - The preview shows the exclusive-`to` rule.
  - The bytes in `recognition.journal` are as expected.
- Renewal prefill.
- Restatement with a closed period: the preview, then the adjustment.
- Invoice match with a fee.
- `sample.journal` (port 5099) shows no Recognition button, section or chip.

**Docs:**

- `docs/journal/accrual-recognition.md` (new):
  - when it appears (the gate);
  - the four revenue shapes and the expense mirror;
  - the tag table;
  - anchored vs unanchored;
  - catch-up and auto-post;
  - restatement and closing;
  - hiding entries;
  - **Check it with hledger**, built from `capture.sh` and covering:
    - `hledger -f books.journal bal` (posted only);
    - `hledger -f books.journal -f recognition.journal reg tag:recognition
      --forecast=tomorrow..` (the schedule ahead, without double-counting
      today, fact 7);
    - the screening query `expr:"not (tag:recognition and
      tag:_generated-transaction)"`, for any `--forecast` run whose period
      starts in the past;
    - `reg tag:recognition=k7f3a2` (one contract end to end);
    - `bal tag:recognized=2026-11 -M`;
    - why `-f` and not `include`, and why not `--budget` (fact 4).
- `docs/journal/invoice-matching.md`, its own page (Q9): settlement links, fees, and
  `hledger bal tag:kpi=receivable not:tag:match` as open invoices.
- Updates:
  - `settings/settings-files.md`: `[books]`.
  - `settings/accounts.md`: `kpi: prepaid`.
  - `journal/problems.md`: the six checks.
  - `budget.md`: recognition rules are never goals.
  - The projections docs: copying the file.
  - `reports/startup-metrics.md`: posted entries carry `customer:`, which
    lifts `billing-clearing`.
  - `development/perf-baseline.md`.
- Screenshots through `web/docs-screenshots`, with a `MANIFEST.md` row.
- `TODO.md`: point the accrual entries here.

## Phase 5 — Later (separate decisions)

- Insights:
  - renewals by month (count and $, from `renewal-date`);
  - new vs renewal ARR from `contract:`;
  - billed / paid / outstanding per customer from settlement links;
  - adjusted burn counting prepaid (Δprepaid), as A1.1 does for deferred.
- N:M settlement, partial payments, and AR/AP aging.
- Customer credit balances from overpayments.
- FX schedules.

## Execution

| Step | Owner                           | Scope                                                         | Gate                                                       |
|------|---------------------------------|---------------------------------------------------------------|------------------------------------------------------------|
| 1    | Engineer A                      | Fixture + capture, Phase 1, bench                             | `just engine-check`, `just engine-test`, the new tests     |
| 2    | Engineer A                      | Phase 2                                                       | Endpoint tests, goldens, `just test-integration`           |
| 3a   | Engineer B (after 2)            | Phase 3, plus the e2e spec written but not run                | `just check`, `just test`, `just lint`                     |
| 3b   | Writer C (parallel with 3a)     | Docs                                                          | `mbr -b docs` is clean                                     |
| 4    | Lead                            | e2e, screenshots, perf figures, full pre-push, diff review    | Everything green                                           |

## Risks

- **Two files, no atomic write.** The source tags are written before the rule
  group (D-editor), and every half-state surfaces as a finding with a fix.
  That's tested by failing the second write.
- **Users including the file anyway**, because the hledger docs teach
  `include`. Mitigated by the finding, the defensive filter, and the file's
  own header comment.
- **Row grouping is a second collapse** next to transfers. Test a group that
  crosses a 200-row block boundary, cursor restore after posting, and a
  transfer pair inside a recognition month.
- **Adoption noise.** "Needs a schedule" on an old QuickBooks import could
  list years of deferrals. That's why it's bounded by the ▾ lookback (default
  90 days), with `recognition: none` as the per-line answer.
- **`basis` inference false positives.** A household account named `…
  payable` turns the helpers on. `basis = "cash"` is the override, and the
  first time the helpers appear the gate's reason is shown ("because
  `liabilities:taxes payable` looks like a payable").

## Open questions

Answered by Patrick, 2026-11-17:

1. **(Q1)** `closed-through` lives in `ledgeline.toml [books]`, written
   only by the explicit **Close books through…** action.
2. **(Q2)** Anniversary timing is **in advance**: the first entry is on the
   start date (Nov 17 start → Nov 17 … Oct 17), and `renewal-date` is
   start + term. Calendar (anchored) entries are dated the 1st (see Q12).
3. **(Q3)** The fee tolerance is **3% of the invoice, no cap**. It's a
   global setting, not per account.

4. **(Q4)** `unearned` counts in the deferred-revenue name inference, and
   `deferred` counts only when the same segment also contains `revenue` or
   `income`, so `liabilities:deferred-tax` doesn't turn the helpers on.
5. **(Q5)** `contract:` takes `new`, `renewal`, `expansion` (the upgrade's
   new line in the credit-back case) and `reactivation` (a renewal after a
   gap).
6. **(Q6)** Posted entries are left out of Review Recents/Unknowns and get
   no `reviewed:` tag.
7. **(Q7)** Insights' Top Transactions collapses a month's recognition
   entries into one line ("Recognition, Nov · $41,250").
10. **(Q10)** Auto-post also runs on the refresh after an external edit
    (e.g. a `git pull` bringing new schedules), not only at load, midnight
    and after a Ledgeline save. It still never posts closed or future
    periods.
11. **(Q11)** Entries hidden by the chip still count in the journal footer
    totals, with an "incl. N hidden" note.

8. **(Q8)** The Recognition overlay opens with `a` ("accruals") on the
   Journal page.
9. **(Q9)** Docs get two new pages under `docs/journal/`: revenue and
   expense recognition, and invoice matching. Each links to the other and to
   `transfer-matching.md`.
12. **(Q12)** Every entry is dated the day its service period begins, the
    day a monthly payer would pay: the anniversary for unanchored contracts,
    the 1st for anchored ones (an opening stub uses its start date). Entries
    are still *posted* at month-end close through **Post through ‹month
    end›**; only their dates follow the period start.

No open questions remain.
