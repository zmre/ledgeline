# 30 — Auto postings, and the rest of hledger's period grammar

Two gaps stop a model that hledger reads correctly from reading correctly
here. Some `~` period expressions that hledger documents fall through to
`PeriodKind::Unsupported`, so their lines silently contribute nothing. And `=`
auto-posting rules are skipped entirely, both in the main journal and in
projection scenarios. This plan closes both. It adds a read-only display of
generated postings in the Journal, and an editing path that sends the user to
the rule that generated a posting.

## The ask (Patrick, 2026-10-27)

> We need to support the hledger auto posting rules across the board, but I'm
> particularly interested in using them purely in forecasting so I can model
> taxes or sales commissions as a percent of revenues … For projection
> purposes, we only want the auto rules to operate on forecasts.
>
> = revenue:sales tag:_generated-transaction
>     expenses:people-costs:sales-commission  *-0.035
>     assets:mercury:checking
>
> [warnings] the period 'every 1st Apr from 2027' is not a recurrence this
> engine can enumerate …
>
> We need UI for these in the projections interface. We should also display
> the auto postings in our journal, but not in an editable way. Or perhaps if
> someone wants to edit an auto generated entry, it takes them to edit the
> generator instead.

## What is true today

- **Period expressions.** `parse::parse_every` (`parse.rs:1799`) knows these
  forms:
  - `every N units`;
  - `every WEEKDAY`;
  - `every Nth day of month|week`;
  - `every Nth WEEKDAY of month`;
  - `every MM/DD`;
  - a bare date;
  - `from`/`to` bounds.

  Every other form becomes `Unsupported`. `forecast.rs:1135` then warns that
  the line contributes nothing, and `BlockLock::UnsupportedPeriod` locks it in
  the budget editor.
- **Auto rules.** The `"="` arm in `parse.rs:702-707` calls
  `skip_indented_block`. No rule reaches `Journal` or `Scenario`, and no
  warning is raised.
  - `ProjectionDoc::apply` copies `=` blocks through byte for byte, because it
    only splices the header and `~` blocks. So nothing is lost on save, but
    the round-trip check never sees them.
  - `parse_amount_and_assertion` cannot read `*0.1`.
  - A `=QUERY` with no space after `=` falls through to
    `UnsupportedDirective`, and that refuses the whole journal.
- **No query engine.** `view/filter.rs` does substring matching and
  `reports::account_matches` does subtree prefixes. Core has no `regex`
  dependency; `regex` appears in `Cargo.lock` only through `criterion`.
- **Generated content has a precedent.** Forecast transactions already carry
  `generated-transaction` / `_generated-transaction` (`forecast.rs:78-81`,
  `is_generated` :339). `ForecastJournalView.svelte` renders them read-only
  with an origin badge. The budget editor's `locked` badge is the read-only
  idiom.

## What hledger 1.52 actually does

Everything below was verified on 2026-10-27 by driving `print --forecast`,
`print --auto --verbose-tags`, `is` and `bal --budget` over scratch journals.
The manual sections that matter are "Auto postings", "Period expressions",
"More complex report intervals", "Multiple weekday intervals" and "Queries".

### Period forms we do not read yet

| Form                                     | Example                          | hledger fires (window 2027)        |
|------------------------------------------|----------------------------------|------------------------------------|
| `every Nth MONTHNAME [of year]`          | `every 1st Apr`, `every 1st July`| 2027-04-01 / 2027-07-01            |
| `every MONTHNAME Nth [of year]`          | `every Nov 15th`                 | 2027-11-15                         |
| `every MM/DD of year`                    | `every 11/15 of year`            | 2027-11-15                         |
| `every Nth day` (no `of month`)          | `every 2nd day`                  | 2nd of each month                  |
| `every Nth WEEKDAY` (no `of month`)      | `every 3rd mon`                  | 3rd Monday of each month           |
| `every weekday` / `every weekendday`     |                                  | Mon–Fri / Sat–Sun, every week      |
| `every D1,D2,…`                          | `every mon,wed,fri`              | each listed weekday                |
| interval `in` period                     | `monthly in 2027`                | 12 occurrences                     |
| `since` (synonym of `from`)              | `monthly since 2027-03`          | from 2027-03                       |
| `..` / `-` as `to`                       | `monthly 2027-01..2027-04`       | Jan, Feb, Mar (end exclusive)      |
| bare date pair (no keywords)             | `2027-01 2027-04`                | (manual; verify)                   |
| quarter literal                          | `2027q2`                         | once, 2027-04-01                   |

hledger rejects these, and they stay `Unsupported`:

- `every nov 15` and `every 15 nov` (the ordinal suffix is required);
- `every last day of month`;
- `every 2nd quarter`;
- `every 2 weekly`.

Quirks to match, not fix:

- `every 29th feb` fires **Mar 1** in a non-leap year.
- `every 5th sun of month` spills into the next month when a month has no
  fifth Sunday: 2027-03-07, 2027-04-04, and so on.
- `every 31st day of month` clamps to the month's end.

Relative dates (`jan`, `this year`, `q4`) depend on today's date. Leave them
out of scope and `Unsupported`, because a projection file must not change
meaning overnight.

### Auto posting semantics

These were verified unless marked otherwise.

1. **Matching.**
   - A rule's postings are inserted **immediately below each posting** the
     query matches.
   - A transaction-level term such as `desc:` matches *every* posting, so the
     rule fires once per posting.
   - Rules apply in file order, and **later rules see earlier rules'
     generated postings**.
2. **Amounts.**
   - A bare `N` takes the matched posting's commodity.
   - `$N` is used as is.
   - `*N` multiplies the matched amount, with an exact product and no
     rounding (`$333.33 * 0.1 = $33.333`).
   - `*N CUR` multiplies and also replaces the commodity.
   - `%account` in an account name expands to the matched account.
   - When the matched posting carries a cost, `*N` scales the cost as well.
3. **Balancing.**
   - One amountless posting in the rule balances against the rule's other
     *real* generated postings. Your
     `assets:mercury:checking` got `$-100` against `$100` of payroll tax.
     (One subagent reported otherwise; my direct test contradicts it.)
   - `[a] $10` / `[b]` balance the same way within the balanced-virtual group.
   - A regular posting left unbalanced is a **hard load error**.
   - Auto postings are added after inference and the balance check, and
     before balance assertions.
4. **Tags.**
   - The transaction gets `modified-transaction:` (visible with
     `--verbose-tags`) and the hidden `_modified-transaction`.
   - Each generated posting gets `generated-posting: = QUERY` and the hidden
     `_generated-posting`.
5. **Forecast only.** `tag:_generated-transaction` (or `tag:generated-transaction`)
   in the query limits a rule to forecast transactions.
6. **File scope.**
   - A rule affects transactions in its own file and in parent or child files
     reached through `include`.
   - It does **not** affect **sibling** `-f` files (hledger #1212). Under
     `hledger -f main -f scenario --auto --forecast`:
     - rules in the scenario apply to the scenario's own `~` forecasts;
     - they do NOT apply to the main file's transactions or `~` rules;
     - the main file's `=` rules do not apply to the scenario's forecasts.
7. **Budgets.** `bal --budget --auto` adds auto postings to *actuals* but not
   to *goals*. A `(expenses:tax) *0.1` rule on wages raised the parent's
   actuals to $1100 against a $1000 goal.
8. **Opt-in.** Rules do nothing without `--auto`.

### Query language

hledger queries are case-insensitive infix regexes (POSIX ERE). These are the
term types and how they combine:

- `acct:` (the default when no prefix is given), `desc:`, `payee:`, `note:`,
  `code:`;
- `tag:NAME[=VAL]`, where tags are inherited from the account, the
  transaction and the commodity;
- `amt:` with `<`, `<=`, `>`, `>=` or `=`. A signed operand compares signed
  values; an unsigned one compares magnitudes;
- `cur:`, which is fully anchored;
- `date:`, `date2:`, `status:`, `real:`, `depth:`, `type:`;
- `not:` negates a term;
- space-separated terms are ORed within description terms and within account
  terms, and everything else is ANDed;
- `expr:'… AND/OR/NOT ( )'` writes the boolean explicitly.

## Decisions (confirmed with Patrick 2026-10-27)

1. **Grammar first, as its own commit series.** Your three warning lines are a
   pure parser gap and fixing it changes no model. It ships before any
   auto-posting work starts.
2. **New period kinds reuse the existing walk.**
   - `every Nth MONTHNAME` and its variants become
     `PeriodKind::Annual { month, day }`, which already exists. Only the
     parser changes.
   - `every Nth day` and `every Nth WEEKDAY` become the existing `Anchored`
     variants.
   - New: `PeriodKind::Weekdays(u8 bitmask)` for `weekday`, `weekendday` and
     comma lists. It walks days and keeps those whose bit is set.
   - `in P` / `since` / `..` / `-` / `2027q2` only change bounds or turn into
     `Once`.

   The budget editor keeps locking every non-bare header (`BlockLock::Period`),
   as it does today.
3. **Auto rules are a first-class model, parsed everywhere.** Add
   `AutoPostingRule { query_raw, query: Query, postings: Vec<AutoPosting>,
   comment, tags, source_span, source_file }`, where an `AutoPosting`'s amount
   is one of:
   - `Fixed(Amount)`;
   - `Bare(Dec)`, which takes the matched posting's commodity;
   - `Scale { factor: Dec, commodity: Option<String> }`;
   - `Inferred`.

   The model also gets `Journal.auto_rules`. A query or posting this engine
   cannot read becomes `Query::Unsupported`. The rule is kept and listed with a
   lock, and it never fires. That is the same "still parsed, still carried,
   locked" contract plan 21 set for period expressions. A malformed `=` block
   never refuses the journal.
4. **The query engine is a core module, `query.rs`, built on the `regex`
   crate.** It is pure Rust, works under the iOS FFI, and supports
   case-insensitive infix matching. POSIX ERE and Rust regex differ only at the
   edges, mainly in backrefs and in how they disambiguate matches; Rust regex
   has no backrefs. A pattern Rust rejects is `Unsupported`.
   - Phase 2 supports `acct`, `desc`, `payee`, `note`, `code`, `tag`, `amt`,
     `cur`, `status`, `real`, `date`, `not:`, the space-separated OR/AND rule
     and `expr:`.
   - `depth:` and `type:` are deferred to `Unsupported`.

   The same module can later serve the journal filter bar (out of scope here).
5. **Applying rules is one pure function**:
   `auto::apply(rules, txn, Scope) -> Transaction`. Both the forecast
   expansion and the real-journal path call it. Generated postings carry a
   **model flag**, `Posting.generated: Option<RuleRef>` (file + block ordinal),
   as well as hledger's tags. The edit path keys off the flag and never off a
   tag in comment text.
6. **Projections: scenario rules apply to that scenario's forecasts only, and
   ALWAYS.** (Patrick, 2026-10-28.) A scenario's `=` rules apply in every
   projection run whatever `hledger.conf`'s `--auto` says. That setting
   (Decision 7) governs only the main journal's real transactions.
   That matches the `-f main -f scenario` command exactly (semantics 6), and
   it is what you asked for.
   - **The writer always includes `tag:_generated-transaction`** in queries
     the UI writes, so stock hledger gives the same numbers with `--auto`
     added to the documented command.
   - **A hand-written rule without the tag** still applies to forecasts only
     in Ledgeline. We add a scenario warning: "hledger --auto would also apply
     this to recorded transactions — add tag:_generated-transaction". The UI
     offers that edit as one click.
   - **Growth transactions** (`ForecastOrigin::Growth`, asset gains) are
     **not** matched. hledger never generates them, so parity holds.
   - **Main-journal `=` rules** do not touch scenario forecasts. hledger
     doesn't apply them either, because the files are siblings.
     `seed_scenario` offers to copy forecast-scoped main-journal rules into a
     new scenario, just as it copies `~` rules today.
7. **Main journal: opt-in, mirroring `--auto`. `hledger.conf` is the setting.**
   - Rules apply to real transactions only when `--auto` is in force in the
     config file's general section. There is no separate Ledgeline
     preference: the file *is* the setting, so the terminal and the app can
     never disagree.
   - The checkbox on Settings › hledger config writes `--auto` into, or
     removes it from, that file (Decision 10).
   - The applied journal is built in `Snapshot::from_journal`.
     `JournalEditor` keeps the **raw** journal, so edit verification,
     elision restore and posting-line mapping see only real postings.
     Toggling the setting republishes the snapshot, which changes the
     generation and so refreshes the client.
   - Balance assertions run on the applied journal when the setting is on,
     as in hledger.
   - The budget gets auto postings on actuals only (semantics 7).
8. **Generated postings are visible and never editable.**
   - Journal rows mark them, and the UI draws them muted with a "⚙ from rule"
     chip.
   - Clicking one, or trying to edit it inline, opens the **rule editor** on
     its block.
   - The transaction modal hides generated postings from the editable form and
     shows them read-only beneath it. `formToBody` never sends them.
   - The server refuses defensively with a 409 naming the rule if a PUT or
     PATCH ever references one.
9. **Hard error parity.** If an applied rule leaves a transaction's regular
   postings unbalanced, it is a load error in hledger. Here it is a
   diagnostic on the transaction plus a lock on the rule, not a refused
   journal. Same proportionality argument as plan 21's Decision 2.

10. **`hledger.conf` is located the way hledger locates it, and gets a GUI.**
    (Patrick, 2026-10-27.)
    - **Lookup order**, nearest first, exactly one file used:
      1. the journal's directory, then each parent directory upward;
      2. `~/.hledger.conf`;
      3. `$XDG_CONFIG_HOME/hledger/hledger.conf` (default
         `~/.config/hledger/hledger.conf`).

      This reverses `hledger_conf::locate_in`'s documented difference #3,
      which skips the home and XDG files today. Steps 2 and 3 only apply on a
      vault with a real directory behind it (the desktop); a sandboxed vault
      stops at its root, as now. The import path's alias forwarding switches
      to the same locator, so both features always read the same file.
    - **Writing:** update the file that was found, wherever it is. If none
      exists, create `hledger.conf` in the **ledger root directory** (the main
      journal's directory), using the existing `new_file_header()`.
    - **Settings › hledger config tab** shows:
      - the path in force, as the existing relative `../hledger.conf` handle
        (or `~/.hledger.conf`);
      - the hijack warning, if the file's first token replaces the command
        (`hijacks_command`);
      - one control per option Ledgeline honours, and nothing for the options
        it ignores.
    - **Options honoured:**
      - general-section `--auto` (checkbox, which also feeds Decision 7);
      - `--depth N` and `--depth REGEX=N` in the general section and in the
        `[balancesheet]`, `[incomestatement]`, `[cashflow]` and `[balance]`
        sections. The command section overrides the general one, as verified
        in hledger.

      **Section names must be hledger's full command names**: verified,
      hledger silently ignores `[bs]`. The GUI writes full names, and it warns
      about an alias-named section it finds, because that section does
      nothing.
    - **What `--depth` drives:**
      - On CF and Net Worth, it becomes the default of the existing depth
        control. A URL `depth=` still wins.
      - BS and IS deliberately have no depth control (plans 12/13). There,
        `--depth` sets the **initial collapse level** of the grouped account
        tree. `REGEX=N` collapses only matching accounts.
    - **Writes are surgical.** Rewrite or insert the one option line, in the
      right section, and leave every other byte, comment and unknown option
      untouched. `with_aliases` already does this for `--alias` and is
      generalized. Writes are refused for a hijacked file (with the existing
      sentence), and anything we can't express is refused rather than
      approximated.
    - **Changing the file republishes the snapshot**, so reports refresh.
      Watch the conf file alongside the journal for external edits.
    - Every hledger subprocess keeps passing `--no-conf`. We read the file;
      we never delegate to it. This is unchanged.
11. **Rule accounts are checked against the accounts list.** A rule posting to
    an account that is neither declared nor used anywhere in the journal (for
    example `expesnses:people-costs:payroll-tax`) gets a warning: on the rule
    in Settings, on the scenario rule in Projections, and as a check in the
    Problems drawer. It is not refused, because hledger accepts it. The
    account pickers in both editors offer only known accounts, with "new
    account" as an explicit choice.
## Phase 1 — Period grammar (fixes the reported warnings)

**`model.rs`**
- Add `PeriodKind::Weekdays(WeekdaySet)`.
- `PeriodSpec::interval()` is unchanged.

**`parse.rs`**
- Strip an optional trailing `of year` or `of month` in `parse_every`.
- Add arms:
  - `[nth, monthname]` and `[monthname, nth]` → `Annual`;
  - `[nth, "day"]` → `Anchored(DayOfMonth)`;
  - `[nth, weekday]` → `Anchored(NthWeekday)`;
  - `weekday`, `weekendday` and comma lists → `Weekdays`.
- `month_name()` accepts full and three-letter names.
- `ordinal` requires a suffix wherever hledger requires one: `every 15 nov`
  stays Unsupported.
- In `split_period_bounds`, accept `since` as `from`, `until` (verify; hledger
  errored on it in one test) and `..` / `-` as `to`, and `in P` as bounds. A
  quarter literal `YYYYqN` becomes a `Once` / bounds date.

**`reports/budget.rs`**
- `occurrences` gains a `Weekdays` branch.
- Check `Annual` against the Feb 29 → Mar 1 quirk and `NthWeekday` against
  the fifth-weekday spill. Fix the walk if the goldens disagree.

**Tests**
- Inline parser tables in `parse.rs` for every form in the table above,
  including the four rejected forms.
- Budget goldens in `tests/budget_golden.rs`, captured from hledger.
- Parity fixture `fixtures/projections-parity/periods.journal` holding your
  three AT&T/Lucid lines verbatim, plus one line per new form. Run under
  `LEDGELINE_HLEDGER_PROJECTION_CHECK=1 cargo test -p ledgeline-core --test
  projection_files`.
- Proptest: `parse_period_spec` never panics, and `raw` always round-trips.

**Docs**
- `docs/projections/file-format.md` and `docs/budget.md`: the supported-forms
  table.

## Phase 2 — Auto rule model, parser and query engine (core only)

**`model.rs`**
- `AutoPostingRule`, `AutoPosting`, `AutoAmount`.
- `Posting.generated: Option<RuleRef>` (about 40 `Posting {` literals to
  update; consider `..Posting::default()` in tests).
- `Journal.auto_rules` (about 65 `Journal {` literals; same).

**`parse.rs`**
- Replace the `=` skip with `parse_auto_rule`, modelled on
  `parse_periodic_transaction` but **without** `balance_postings`.
- Accept `=QUERY` with no space.
- Posting amounts may be `*N`, `*$N`, `*N CUR`, a bare `N` or an amount.
- Delete the test `periodic_rule_parsed_but_excluded_from_transactions` (its
  `=` half) and replace it.

**`query.rs` (new)**
- Tokenizer that handles single and double quotes, the `not:` prefix and
  `expr:`.
- `Query` AST.
- `fn matches(&Query, &Transaction, &Posting, &InheritedTags) -> bool`.
  Inherited tags reuse `wire::posting_tags`.
- The `regex` crate goes in workspace deps, with a case-insensitive builder
  and a size limit (DoS guard, alongside `parse_resource_limits.rs`).

**`auto.rs` (new)**
- `apply(rules, &Transaction, Scope) -> Transaction`, implementing semantics
  1–4:
  - insert below the match;
  - later rules see earlier output;
  - per-rule inference of the one amountless posting;
  - `%account` expansion;
  - cost scaling;
  - tags plus the `generated` flag.
- Scale with `Dec::mul` with **no** precision rounding.
- `Scope` is `ForecastOnly | All`.

**Tests**
- Unit tables for every query term against hledger-captured expectations.
- `tests/auto_postings_golden.rs`: fixture journals compared against
  `hledger print --auto -O json`, opt-in like the projection parity suite.
  Pins ordering, per-posting `desc:` firing, chained rules, the multiplier's
  exact scale, commodity replacement, amountless balancing, `%account`, and
  virtual / balanced-virtual groups.
- Proptest: `apply` with zero rules is the identity, and the regular postings
  of the output balance whenever every rule is self-balancing.

## Phase 3 — Projections engine and file

**`projections.rs` / `serialize.rs`**
- `Scenario.auto_rules: Vec<ScenarioAutoRule>`, carrying the block index for
  splicing.
- `read_scenario` keeps the rules whose `source_file` is the scenario file.
- `ProjectionDoc` learns to locate, splice, append and delete `=` blocks. It
  follows the same discipline as `~` blocks: rewrite only the blocks that
  changed. Today `=` blocks are copied verbatim, so this is additive.
- `confirm_round_trip` compares rules too.
- `chunk_print` renders a rule. Queries written by the UI always end in
  `tag:_generated-transaction`.

**`forecast.rs`**
- In `expand_from`, after `revalue_priced_rows` (:1015) and before the date
  sort, apply `auto::apply(.., Scope::ForecastOnly)` to transactions whose
  origin is `Rule` or `Event`, and skip `Growth`.
- Extend the commodities set so the mismatch warning covers generated amounts.
- Add a warning for an untagged query (Decision 6) and one for a locked or
  unbalancing rule.
- Plan 29's cache is untouched: the rules depend only on the scenario.
  `project_cached` and the reference path go through the same `expand_from`,
  so the equivalence suite needs only a new auto-rule scenario.

**Virtual auto postings and the cash flow statement**
- Plan 28 amendment 26 turns virtual scenario lines into regular postings
  because `cash_flow_statement` drops virtual postings. A `(liabilities:tax)`
  auto posting would vanish from projected CF but show in hledger `cf`.
- **Decided:** line up with hledger. Keep the virtual type, and make
  `cash_flow_statement` include virtual postings, as `hledger cf` does
  without `-R` (verified; see "Resolved" item 1).

**Tests**
- `projection_files.rs`: add `--auto` to `documented()`'s argv, and add a
  parity fixture `fixtures/projections-parity/auto.journal` containing your
  two rules verbatim (sales commission and payroll tax).
- `projection_cache.rs`: an auto-rule scenario in the equivalence and proptest
  set.
- `serialize.rs`:
  - byte preservation of untouched `=` blocks;
  - editing one rule rewrites only its block;
  - a new rule is appended;
  - deleting a rule removes its block exactly;
  - a hand-typed rule with an odd query round-trips.

## Phase 4 — Projections server and UI

**Server (`projections_api.rs`)**
- `WireScenario.autoRules`, with every field emitted as `null`, never absent.
- `ScenarioIn.autoRules` uses `deny_unknown_fields`. It is optional on the
  way in, so older clients still work.
- Add `MAX_AUTO_RULES` and `MAX_AUTO_POSTINGS` beside the other caps, checked
  before decode.
- Validation:
  - the query parses;
  - every posting is readable;
  - at most one amountless posting per balance group;
  - regular postings net to zero, or exactly one amountless regular posting
    exists. Otherwise refuse with a sentence, so that the file stays
    hledger-loadable.
- `seed` offers forecast-scoped main-journal rules.

**Goldens**
- `fixtures/native/v1/projections-*.json`, `projection_endpoints.rs`,
  `nativeDecode.test.ts` (`DECODERS`), and `nativeLive.integration.test.ts`.
- Run `just test-integration`.

**UI (`ProjectionsTable.svelte`)**
- Add a new section, **"Automatic postings"**
  (`data-testid="projection-section-auto"`), after One-off events. Each row is
  one rule.
- **When:**
  - an account picker, defaulting to `acct:^…$` exact;
  - optional extra query terms in a free-text "advanced" field;
  - the forecast-only scope is implicit and shown as a fixed chip.
- **Posts:** one or more rows of account + amount mode (`% of matched`,
  `fixed`), with sign handled like the other sections.
  - `% of matched` is entered as `3.5` and stored as `*-0.035`. The sign is
    inferred from account types so the user types positive numbers, reusing
    `DecodedScenario::signed`.
  - An optional counter-account, defaulting to the scenario's cash account,
    is the amountless leg.
- **Description and comment.**
- A preview line under each rule, for example "≈ $8,750 on each $250,000
  AT&T sale", computed from the current run.
- A locked rule (with an unsupported query) is shown read-only with its raw
  text and the lock reason, following the budget editor's pattern.
- `ForecastJournalView.svelte` marks generated postings with the rule chip.
  Clicking the chip scrolls to the rule row.
- Model plumbing: `types.ts`, `scenarioModel.ts` (`emptyScenario`,
  `cloneScenario`, `scenarioToWire`, `projectable`), `native.ts` and
  `nativeDecode.ts`. Mind the NUL byte in `scenarioModel.ts`: use `grep -a`.

**Tests**
- `scenarioModel.test.ts`, `ProjectionsTable.svelte.test.ts` and
  `nativeDecode.test.ts`.
- e2e: a read-only test that loads a fixture scenario with a rule and asserts
  that the section renders with a lock or chip. Do not assert figures; that
  is the existing e2e rule.

## Phase 4b — `hledger.conf`: locate like hledger, and a Settings tab

This phase is independent of the auto-rule phases and can run in parallel
with Phases 2–4. It must land before Phase 5, which reads `--auto` through
it.

**Core (`hledger_conf.rs`)**
- `locate_conf` gains the home and XDG fallbacks after the upward walk
  (Decision 10). `$HOME` and `$XDG_CONFIG_HOME` are passed in as parameters,
  never read from the environment inside core, which keeps core pure and
  testable. The server supplies them from its existing AppDirs seam.
- Rewrite the module doc's "three deliberate differences" section: only
  "starts at the journal's directory, not the process cwd" remains.
- `ConfFile` gains a `Home` / `Xdg` variant (or a `Located { path, origin }`).
  The handle shown to the client is `~/.hledger.conf` or
  `~/.config/hledger/hledger.conf`, never an absolute path (security layer 5,
  as `conf_id` does today).
- New `ConfSettings { auto: bool, depth: BTreeMap<Command, DepthSpec>,
  alias_sections: Vec<String> }`. It is a pure reading of `parse()` output,
  and the command section overrides the general one.
- `DepthSpec` is `{ all: Option<u32>, per_regex: Vec<(String, u32)> }`.
- Generalize `with_aliases` into `with_option(text, section, option, value)`
  and `without_option(...)`. They insert into the right section (creating the
  `[command]` heading at EOF if absent), replace an existing line in place,
  and preserve CRLF, comments and unknown options. Hijacked files are
  refused.

**Server**
- `GET /api/hledger-conf`: returns the handle, whether it exists, the
  hijacked sentence or null, the `ConfSettings`, warnings (alias-named
  sections), and a revision.
- `PUT /api/hledger-conf`: sets `auto`, sets or clears `depth` per command.
  It writes to the located file, or creates `<ledger root>/hledger.conf`. It
  uses conditional writes on the revision (409 on conflict) and takes the
  same write lock as alias writes.
  - The existing `/api/import/hledger-conf` alias route keeps working but
    moves onto the shared locator.
- A write, or a watcher-detected external change, republishes the snapshot.
- `ConfSettings` is held in `Snapshot`. Reports read it for default depth;
  Phase 5 reads `auto`.

**Reports**
- CF and Net Worth: `--depth` is the default when the URL has no `depth=`.
- BS and IS grouped trees: initial collapse level, with `REGEX=N` per account.

**Web**
- `SettingsTabs.svelte` gets an **hledger config** tab with:
  - the file handle, or "none — will be created at <root>/hledger.conf";
  - the `--auto` checkbox;
  - a depth field per report (BS, IS, CF, Balance), each with optional
    per-account overrides;
  - the hijack and alias-section warnings.
- `params.ts` takes server-supplied defaults for `depth`.

**Tests**
- `hledger_conf.rs` units:
  - locator order across all three tiers, with tempdir `$HOME` / XDG;
  - command section overrides general;
  - `[bs]` is ignored and warned;
  - an insert into a missing section creates it;
  - CRLF is preserved;
  - a no-op write is byte-identical.
- Opt-in hledger cross-check: write a conf with the GUI path, then assert
  `hledger bs` (without `--no-conf`) shows the depth we apply.
- Server endpoint tests: create-in-root, update-where-found, 409 on stale
  revision, refusal of a hijacked file.
- Docs: `docs/settings/hledger-config.md`.

## Phase 5 — Main journal: apply, display, redirect edits

**Setting**
- `--auto` in the general section of the `hledger.conf` in force, read
  through Phase 4b's locator. There is no other switch.
- Rules live on Settings › **Automatic postings** (see "Rule editor" below).
  That tab repeats the `--auto` checkbox from the hledger config tab, bound to
  the same endpoint, so the rules and the switch that activates them sit
  together.

**Snapshot (`ledgeline-server/src/lib.rs`)**
- `Snapshot::from_journal` derives `applied: Arc<Journal>` when the setting is
  on. Reports, views, insights, holdings and checks read `applied`.
  `JournalEditor` stays raw. The FFI gets this for free through `AppState`.
- Audit the consumers listed in the journal-view map, in particular these:
  - `subscriptions.rs`: generated postings must not create phantom recurring
    charges, so exclude `generated` postings from recurrence detection;
  - `journal_meta` display precision: exact products such as `$33.333` must
    not widen display precision, so exclude generated amounts from style
    inference;
  - `edit_api::find_style_for`: same.

**Rows and wire**
- `RowPosting.generated: {file, block, query}`, skipped when absent.
- The hledger-compatible `/transactions` wire stays raw unless the setting is
  on, in which case it emits `ptags` with `generated-posting` as hledger does.

**Journal UI**
- `AccountsCell` and `txnFlowAmounts` ignore generated postings for the
  source→destination chips and add a small "+N auto" badge. A two-leg sale
  stays readable.
- The expanded or modal view lists generated postings muted, with "⚙ from
  rule `= revenue:sales`".
- **Edit redirect:**
  - an inline account edit on a generated posting, or clicking its chip,
    navigates to Settings › Automatic postings with that rule focused;
  - `accountPatch` maps UI positions to raw positions using the flag;
  - `txnToForm` drops generated postings;
  - the server returns 409 if one ever arrives.

**Rule editor (Settings › Automatic postings)**
- Span-based, like `periodic.rs` / `aliases.rs`: rewrite only the edited
  block's bytes, then verify by re-parse.
- Lists every rule in every journal file with its source location. Supports
  editing the query and postings, and adding or deleting a rule.
- Same lock pattern for anything unsupported.

**Tests**
- `journal_view_golden.rs` with an auto fixture.
- `edit_guards.rs`: PUT/PATCH on a generated posting returns 409, and a PATCH
  on a real posting after a generated one hits the right line.
- `balance_assertions.rs`: assertions are evaluated after application.
- Reports goldens with `--auto` from hledger.
- The `subscriptions` exclusion.
- Web tests for `AccountsCell` and `editMapping`.

## Sequencing

1. **Phase 1.** It's independent, small, and fixes the warnings you reported
   today. Ship it as its own PR.
2. **Phase 2 → Phase 3 → Phase 4.** This is the forecasting value you asked
   for first, and it is one PR or a stacked pair.
3. **Phase 4b** in parallel with 2–4. It's its own PR.
4. **Phase 5**, after 4b. It has the largest blast radius (every report) and
   is active only when `--auto` is in force. It's its own PR.

Worktree agents branch from `main`, so after merging, re-run the full suite
(`just pre-push`).

## Resolved with Patrick (2026-10-27)

1. **Virtual auto postings in projected cash flow:** line up with hledger.
   Verified 2026-10-27: `hledger cf --auto` **includes** a virtual
   `(assets:checking:reserve) *0.1` posting (−$10). Only `cf -R` drops it, and
   `bs` includes it. Ledgeline's `cash_flow_statement` skips virtual postings
   (`cash_flow_statement.rs:526`), so lining up means changing that.
   - **Scope:** make `cash_flow_statement` include virtual postings by
     default, as hledger does. This also changes the real-journal CF for
     anyone who writes `(…)` postings to cash accounts today; that is a
     parity fix, not a regression. Reconcile with plan 28 amendment 26 (which
     regularized virtual scenario lines only to dodge this skip). Those lines
     can stay regular; nothing gains from re-virtualizing them.
   - Pin this with a `tests/cash_flow_statement.rs` case captured from
     hledger, plus a projection parity case. It lands in Phase 3.
2. **Main-journal default:** `hledger.conf`'s `--auto`, with a GUI on a new
   Settings tab, plus `--depth` per report section (Decision 10, Phase 4b).
   Use hledger's lookup order. Create the file in the ledger root if none
   exists; otherwise update the one found.
3. **Rule editor home:** a Settings tab, which also carries the `--auto`
   checkbox.
4. **Typo guard:** warn when a rule references an account not in the
   accounts list (Decision 11).

## Status

- **Phase 1: built 2026-10-27.** Not committed yet.
- **Phases 2–3 and the wire half of 4: built 2026-10-28.** Not committed
  yet. Covers the `=` model and parser, `query.rs`, `auto.rs`, projection
  application, writer preservation and Save As, `autoRules` on the API, a
  read-only list on the Projections page, and the "auto" marker in the Journal
  tab.
- **Phase 4's rule EDITOR: built 2026-10-28** (amendments 12–15):
  `auto_form.rs`, in-place `=` block writes, `form`/`block` on the wire,
  Decision 11's warning in projection runs, and `lib/autoRules/
  AutoRuleEditor.svelte` in the Projections table.
- **Phase 4b (`hledger.conf` tab): built 2026-10-28.** Core locator and
  settings, `GET`/`PUT /api/hledger-conf`, Settings › hledger config, the
  report defaults, and `docs/settings/hledger-config.md`. See amendment 4b.
- **Phase 5 (the main journal): built 2026-10-28**, minus rule EDITING in
  Settings (amendments 16–21). Covers the `Posting.generated` marker,
  `auto::apply_journal` in the snapshot under `--auto` (read from Phase 4b's
  `ConfSettings`), rows and the SPA marking generated postings, the 409s,
  served-to-written PATCH mapping, the consumers (styles, subscriptions,
  assertions, budget actuals), the `auto-rule` / `auto-account` diagnostics,
  `GET /api/auto-rules` and a read-only Settings › Automatic postings tab
  whose `editor` snippet prop is where the shared rule editor plugs in.
- **Rule editing in Settings: built 2026-10-28** (amendment 22).
  `auto_edit.rs`, `PUT /api/auto-rules`, `AutoRuleForm::forecast_only`, and
  add/edit/delete on the Settings tab (the `editor` seam is gone; the panel
  edits).

## Contract amendments made during implementation

### 1. Phase 1 needed no `PeriodKind::Weekdays`; it is `Anchor::Weekdays(u8)`

A weekday set is one more anchor of the existing weekly family. It snaps
`from` down like `every tuesday` does, and the walk is shared
(`budget::weekday_dates`). A new `PeriodKind` would have meant a second weekly
code path.

### 2. hledger's bound grammar is narrower than the manual suggests

Verified 2026-10-27, and encoded in `parse::parse_bounds`:

- `until` is legal ONLY as an end with no start (`monthly until D`).
  `from D until D2` and `D until D2` are hledger errors.
- `..` and `-` work as `to` between dates, and as an open start
  (`monthly .. D`).
- `in` takes a single date only. `in D1..D2` is an error.
- A lone date after an interval bounds the rule to that date's span:
  `monthly 2027-03` fires once.
- Dates may be compact (`202703`, `20270301`) or joined to a range with no
  spaces (`2027-01-2027-04`).

`period_head_len` was added so that `PeriodSpec::interval_words` cuts at the
parser's own boundary instead of scanning for `from`/`to`. The bounds no
longer always begin with one of those words.

### 3. Yearly dates overflow on the first occurrence, then clamp

`every 30th feb` fires Mar 2 and `every 2/29` fires Mar 1 in a non-leap year
(even in 2028, after a Mar 1 2027 start). `every 2/29 from 2028` fires Feb 29,
then Feb 28. Rule: the first occurrence on or after the base is built with
the day overflowing its month, and each later one is that date plus whole
years with the day clamped. `budget::annual_dates` now does exactly this; it
previously clamped every year.

### 4. Two pre-existing over-acceptances closed

hledger rejects `every 2 weekly` (a count before an `-ly` word) and
`every 2/30` / `every 4/31` (an `MM/DD` that never exists). Both were
previously read: the first as every two weeks, the others clamped. Both are
now `Unsupported`, as in hledger.

### 5. Known divergence kept: a fifth weekday

`every 5th sun of month` in hledger spills into the next month, and the spill
pattern depends on `from` in a way that could not be pinned from black-box
runs. Ledgeline still skips months with no fifth such weekday, as it did
before plan 30. This is documented in `docs/budget.md`. Only nth = 5 is
affected.

### 7. `=QUERY` with no space no longer refuses the file (2026-10-28)

hledger reads `=revenue:other` as `= revenue:other`. Here it fell through to
"unsupported directive", which made a whole scenario or journal unreadable.
It is now skipped like any other `=` block until Phase 2 models it. Pinned by
`parse::tests::an_auto_rule_with_no_space_after_the_equals_sign_does_not_refuse_the_journal`.
`projection_files.rs::a_save_keeps_every_auto_posting_rule_byte_for_byte`
pins that a scenario save keeps every `=` block intact.

### 8. hledger infers an amountless auto posting per TRANSACTION, not per firing (2026-10-28)

A rule with an amountless leg that matches twice in one transaction (two
child accounts in one `~` block) adds that leg twice, and hledger 1.52 then
refuses the file with "there can't be more than one real posting with no
amount". Ledgeline balances each firing on its own instead, which is the
reading `*-N` on that leg spells out. It warns, through
`auto::hledger_refuses`, with that fix. One firing per transaction (the user's
case, one person per `~` rule) reads identically in both.

### 9. A bare number in an auto posting stays commodity-less (2026-10-28)

The manual says `2` takes the matched posting's commodity. The 1.52 binary
writes `2` beside a `$5` posting, so the binary wins. `AutoAmount::Bare` was
dropped; a bare number is `Fixed` as parsed.

### 10. Rules travel as their raw text

`WireScenario.autoRules` is `[{raw, query, unsupported}]`, and inbound only
`raw` is read. It is checked to be one `=` line followed by indented lines
with no blank line BEFORE the parser sees it, so a rule body cannot smuggle in
`include` or a transaction. The writer appends any rule the file lacks (Save
As) and `confirm_auto_rules` refuses a save that would lose one.

### 11. The IS parity harness compares against `hledger is --tree`

Since IS rows became subtree figures (the 2026-10-28 roll-up fix), the flat
`hledger is` is no longer like for like once a parent and its children both
post. `hledger_is_rows` reads `--tree` and rebuilds full names from its
indentation. It compares only the accounts the flat report lists, which are
exactly Ledgeline's rows. `auto.journal`'s per-person wages are the first
fixture to exercise this.

### 6. The parity guard compares against hledger directly

`budget::tests::every_period_form_fires_on_hledger_s_dates` pipes each of 70
accepted forms and 14 rejected forms to `hledger print --forecast` and
requires identical dates (more than 2,000 compared), or a refusal on both
sides. It is wired into `just hledger-checks`. `fixtures/projections-parity/
periods.journal` adds statement, Accounts and Journal parity for your three
reported lines.

### 12. A rule's identity is its block, and a save edits `=` blocks in place (2026-10-28)

`Scenario.auto_rules` is `Vec<ScenarioAutoRule { block, rule }>`, where `block`
is the rule's position among the scenario FILE's own `=` blocks when it was
read (the Phase 3 plan's "carrying the block index for splicing"). The wire
carries it both ways. `ProjectionDoc::apply` now splices `=` blocks the way it
splices `~` blocks:

- A rule claims a block in three passes, most certain first: its own block if
  that block still holds exactly its text; else any unclaimed block holding
  exactly its text; else (an edited rule) the block it names, if free. The
  second pass is what keeps bytes in place for a body with no blocks (a client
  from before this amendment) and for STALE block numbers: the page keeps its
  own scenario across a save (`markSaved`, never `adopt`), so after deleting
  the first rule the others still carry their old numbers.
- Claimed blocks are slots filled in the scenario's order (hledger applies
  rules in file order, and later rules see earlier output); leftovers are
  appended; unclaimed blocks are removed with one blank line (the following
  one, or the preceding one at EOF).
- A slot whose bytes already state its rule is not spliced, so untouched rules
  keep their layout, comments and CRLF.
- `confirm_auto_rules` now requires EXACTLY the scenario's rules, in order.
  It used to require only that each was present somewhere.

### 13. Sign inference relates two accounts by their normal side (2026-10-28)

The rule editor takes positive percentages meaning "the target account grows".
`factor = percent / 100 × side(matched) × side(target)`, with side `+1` for
asset, expense and untyped accounts and `−1` for liability, equity and revenue
(gains and conversion included), from `AccountTypes::resolve`. A fixed amount
takes `side(target)` alone.

This deliberately differs from the table's own entry rule (`signed_entry`,
which negates revenue only). A table line is one account's movement; a rule
relates two, and "grows" is the only reading under which the same typed `3`
means the same thing whichever way round they are. Pinned: revenue→expense
`*-0.03`, expense→expense `*0.1`, expense→liability `*-0.1`
(`auto_form::tests`).

The types used are the MAIN journal's declarations, on both the outbound
`form` and the inbound render, so they always agree with each other. A type a
scenario file declares for itself that the main journal does not (and that
name inference does not give either) is not seen by sign inference. That case
is rare and is not handled.

### 14. What the form can state, and how it is written (2026-10-28)

`AutoRuleForm { match_account, extra_query, postings: [{account,
Percent | Fixed{commodity, quantity}}], counter, comment }`. `render` writes
`= ^ACCOUNT(:|$) tag:_generated-transaction EXTRA  ; COMMENT`, with the account
regex-escaped and quoted when it holds a space. Both forms were verified to
match literally in hledger 1.52. Percent legs are `*N`. The counter is the
OPPOSITE multiplier (or negated fixed sum in one commodity), never amountless
(amendment 8). A mix of percentages and fixed amounts with a counter is
refused. `render` re-parses its own output and requires it to read back as a
form rendering to the same text. A proptest pins
`form_from_rule(parse(render(f))) == f`.

`form_from_rule` also accepts a plain account word and an amountless last
posting, and tolerates a missing forecast tag, so the user's hand-written
rules open as forms. Editing one rewrites it canonically: anchored and tagged.
The anchoring is a real change for a sibling such as `revenue:sales2`, which
the infix query matched. The UI says nothing about that edge. Everything else
(virtual or status-marked postings, posting comments, `*N CUR`, a cost,
`%account`, multi-line comments, other account-term shapes, an unsupported
query) is `None`: shown raw, locked, still carried and applied.

Wire: `WireAutoRule` is `{raw, query, unsupported, form, block}` (all
present). `AutoRuleIn` takes exactly one of `raw` and `form`, plus `block`. A
form is decoded at request time (`MAX_AUTO_POSTINGS = 50`, byte caps on its
text fields) and rendered in `DecodedScenario::signed` under the declared
types, then validated exactly as raw text is. The client sends a rule's form
only once it is edited (`AutoRule.edited`, UI-only) or new. An untouched rule
goes back as its raw text, so its bytes never move.

### 15. Decision 11 in projection runs (2026-10-28)

`auto_form::unknown_account_warnings` names each rule account that is neither
in `known` nor a parent of something in it. For a run, `known` is
`JournalFacts::known_accounts()` (every declared account and every account a
real transaction uses) plus every account the scenario itself names
(`Scenario::named_accounts`). `%account` is skipped. It is a warning in the
run, not a refusal. The Settings and Problems-drawer halves of Decision 11
belong to Phase 5.


### 4b. Phase 4b as built (2026-10-28)

Verified against hledger 1.52 with scratch journals and scratch config files
before any of this was written:

- **Section headings are case-sensitive.** `[BalanceSheet]` is ignored like
  `[bs]`. The "alias section" warning covers both.
- **Only the LAST heading of a name is read.** `[balance]\n--depth 3\n[balance]\n--no-total`
  applies no depth. `ConfSettings` reads, and `with_option` writes, the last
  one, and a duplicate is warned about.
- **Plain and per-regex depths combine.** The last plain `N` wins, and the last
  value for one REGEX wins. `-N` means `--depth N`. General options apply first,
  then the command section's; that is what "command section overrides general"
  means for depth.
- **Which of several matching REGEXes wins** is the one matching furthest right
  in the account name, regardless of order. `broker=3` beats `inv=4` for
  `assets:inv:broker:fund`, and `fund=4` beats `assets=2`. A REGEX is matched
  case-insensitively against the FULL name: `inv$` does not match
  `assets:inv:broker`. `reports/ui/confDepth.ts` implements this for the
  initial expansion.
- **An invalid REGEX makes hledger refuse every command**, so writes refuse any
  pattern the `regex` crate rejects, and reads warn about it.
- **A `--depth` with its value on the next line** is read by hledger. We report
  it as unreadable rather than following it, a deliberate narrowing nobody
  should hit.

Decisions made while building:

- **API.** `locate_conf_for(vault, dir, &UserConfDirs) -> LocatedConf { source,
  origin }` is the full lookup. `locate_conf`, `parse`, `read` and `read_in` are
  unchanged and source-compatible, so `locate_conf` alone still skips the home
  and XDG tiers. Phase 5 should read `AppState::conf_settings()`, or
  `locate_conf_for` with `AppDirs::hledger_conf`, rather than `locate_conf`.
  `auto_enabled(text)` and `ConfSettings::auto` are its flag.
- **Where the user tiers come from.** `AppDirs::hledger_conf` (`$HOME`,
  `$XDG_CONFIG_HOME`) is filled only by `AppDirs::from_env()`, which only the
  desktop binary hands to `AppState::with_app_dirs`. A library caller, including
  every in-process test, starts with it empty. `LEDGELINE_IGNORE_USER_HLEDGER_CONF`
  keeps the live-integration, snapshot and e2e servers, and the CLI import
  tests, off the developer's own dotfiles.
- **Net Worth reads `[balance]`.** Net worth is hledger's `balance
  --historical` over assets and liabilities. Cash Flow reads `[cashflow]`. Their
  single depth control uses the plain depth only. Per-regex depths there are
  written for the terminal and noted on the tab.
- **BS/IS initial expansion.** A group opens when hledger at that depth would
  show its accounts as more than one line. With no depth configured, every
  group starts collapsed, as before. A section with only REGEX depths leaves
  unmatched accounts unlimited, as hledger does, so their groups open. Each group
  is decided once per spec, so a collapse sticks across refetches.
- **The snapshot holds the conf lazily** (`Snapshot.conf: OnceLock`), read
  through `AppState::conf()`. A PUT republishes with the written conf. A GET
  reads fresh, and republishes when the file differs from what the snapshot
  holds; this catches `~/.hledger.conf` and parent folders, which no watcher
  covers. `refresh()` does the same check, and the file watcher now watches the
  conf in force and `<root>/hledger.conf`.
- **Writes.** They go to the located file wherever it is, or create
  `<root>/hledger.conf` with `new_file_header()`, whose wording now covers both
  writers. A file outside the vault is written through a `LocalVault` rooted at
  its own folder, so the pinned read and replace are the same as for every other
  write. A symlinked dotfile is shown read-only, never written through.
  General-section inserts now go after the last general option, not after the
  blank line before the first heading. The alias fix's insert changes the same
  way.
- **Not done:** the alias fix (`POST /api/import/hledger-conf`) still writes
  only `<root>/hledger.conf`, as before. Only its READ moved to the shared
  locator. `alias_arguments` still merges every `[import]` heading,
  case-insensitively, where hledger reads only the last exact one. That is a
  small parity gap, left for a follow-up.

### 16. PATCH positions are the SERVED ones; the engine maps them (2026-10-28)

The plan had `accountPatch` map UI positions to file positions in the SPA.
Instead `PATCH /api/transactions/{index}` takes the position in the
transaction as served (generated postings included), and
`edit_api::written_positions` maps it under the editor lock, from the
editor's own transaction re-applied with `auto::apply_in`. A position landing
on a generated posting is a 409 naming the rule. One mapping, next to the
file it describes, for every client (the SPA and the FFI alike); the SPA's
`accountPatch` only skips generated postings. With `--auto` off the two
coincide. A POST/PUT posting carrying `generated` is refused the same way.

### 17. `RowPosting.generated` is `{rule, query}` — no file

A row never carries a path. `rule` is the position in `Journal::auto_rules`;
`GET /api/auto-rules` gives each rule's file relative to the journal and its
line. `Posting.generated` (the model) is `Arc<RuleRef { rule, source_file,
line, query }>`.

### 18. "Unbalanced" reads written postings; a rule's imbalance is `auto-rule`

`check_transaction_balances` skips generated postings (hledger checks balance
before auto postings), and `check_generated_balances` checks the generated
ones on their own. A rule that unbalances a transaction is therefore reported
once, as an `auto-rule` error naming the rule, not twice. An amountless leg
firing twice (`hledger_refuses`) is also `auto-rule`; a rule account nothing
else declares or uses is an `auto-account` warning on that account (Decision
11). Verified 2026-10-28: hledger refuses `= revenue:sales / liabilities:tax
$10` with "The real postings' sum should be 0 but is: $10".

### 19. `tag:` sees account-inherited tags; commodity tags do not reach rules

Verified 2026-10-28 with `hledger print --auto`: `= tag:taxable` fires on
`expenses:food:lunch` under `account expenses:food ; taxable: yes`, while
`= tag:kind=cash` does not fire through `commodity $1.00 ; kind: cash`.
`apply_journal` passes the account tags (ancestors included); the projection
path keeps passing none.

### 20. Only the GENERAL section's `--auto` counts

`[balance] --auto` would apply to `hledger balance` alone; Ledgeline has one
journal for every report, so only a general-section `--auto` turns rules on.
A hijacked conf (first word not a flag) turns them off, as it does in hledger.

### 21. The fixture is `fixtures/auto-postings/`

The global `*-journal` git ignore hides a directory named `auto-journal`.

### 22. Settings edits the main journal's rules; the form says whether it is forecast-only

Built 2026-10-28.

- **Forecast-only is a form field.** `AutoRuleForm::forecast_only` replaces
  the always-written `tag:_generated-transaction`. A main-journal rule usually
  fires on recorded transactions. Ledgeline forecasts nothing in the main
  journal, and main-journal rules never reach a scenario (hledger #1212), so
  a forecast-only rule there never fires in Ledgeline. Reading back, the tag
  (or its visible twin) sets the field. A scenario's rules stay forecast-only
  whatever the client sends: `projections_api` forces the field before it
  renders.
- **`auto_edit`** (core) is the span editor: `block_spans` from the parser's
  `source_span`, which the scenario writer now shares, and `apply` for
  replace, remove (taking one now-doubled blank line with it) and append
  (after a blank line). `expected_raws` is what the re-parse must read back.
  Verified 2026-10-28 that hledger applies a rule to transactions above it
  in the file, so appending to the main file is safe.
- **`PUT /api/auto-rules`** follows the budget writer's pipeline, reusing
  its helpers. The body is `{journalId, revision, change}`, where `change`
  is one of `add {rule}`, `replace {block, was, rule}` or
  `remove {block, was}`, and `rule` is `{form}` or `{raw}`. `was` catches a
  moved block even under a matching revision. The spans come from a parse of
  exactly the bytes read, never the snapshot's. The answer is the new
  listing.
- **The listing** gains `editable`, `newRule` (the main file and its
  revision), and per rule `journalId`, `block`, `revision` and `form`. The
  form wire types and `rule_from_text` / `form_from_wire` moved from
  `projections_api` to `auto_wire`, shared by both endpoints.
- **The panel** edits one rule at a time and saves only on Save, never per
  keystroke: a half-typed rule in the journal would change every report.
  `AutoRuleEditor` takes `scope: "journal"`, which shows a forecasts-only
  checkbox instead of the badge.

