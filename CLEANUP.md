# Cleanup backlog

Findings from the max-effort code review of the `2026-10-27-auto-postings` branch (diff
`main...HEAD`, 2026-10-28). Sixteen finder angles produced 111 candidates; 84 distinct mechanisms
were each verified by a dedicated agent (80 CONFIRMED, 4 PLAUSIBLE), most reproduced against
`hledger 1.52` from the dev shell, probe builds of the crates, the built server, or scratch vitest
runs. The 13 high-severity findings were fixed on the branch before the PR (see the end of this
file). This file holds the 43 medium and 28 low findings that were not, so nobody has to
re-investigate them. IDs are the review's own and stable; `file:line` refers to the branch tip
before the fixes landed (commit `601c4d0`), so lines may have drifted slightly.

How to read an entry: the first line is the ID, location and verdict; then the defect; then
*Scenario*, a concrete reproduction; then *Evidence*, the verifier's proof trail with the exact
lines it read and what it ran. PLAUSIBLE means the mechanism was traced in code but not executed.

One-line synthesis: most of what is below is Ledgeline reading or writing an `=` rule, a query
term, or `hledger.conf` in a way that is almost, but not exactly, what hledger does, and doing so
silently. Where a fix is cheap, it is usually "call the helper that already does it right".

## Medium (43)

### Engine (crates/ledgeline-core)

- **A1-4** `crates/ledgeline-core/src/auto.rs:486` (CONFIRMED)
  A rule with two amountless postings in one balance group is refused by hledger 1.52 even when it
  fires once ('There can't be more than one real posting with no amount'), but Ledgeline parses it
  without complaint, generate() gives EACH amountless leg the full negated sum of the group's fixed
  legs, and hledger_refuses only tests fired > 1 — so the user gets an Unbalanced finding whose
  advice ('leave one of them without an amount') describes what they already did, or, when the fixed
  legs net to zero or there are none, NO finding at all while two fabricated $0 legs are shown.
  Hand-written rules only (the form allows one counter).
  *Scenario:* `= expenses:food / expenses:tax *0.1 / liabilities:tax / assets:other` over
  `expenses:food $10`: generated $1, $-1, $-1; findings = Unbalanced $-1 with wrong advice; hledger
  refuses. Variant with no fixed legs: both inferred legs $0, no finding, journal reads valid;
  hledger refuses identically. Same for balanced-virtual groups.
  *Evidence:* auto.rs:473-497 (each Inferred leg skips other Inferred legs), 249-254, 314-324;
  parse.rs:1675-1736 no guard; grep of AutoAmount::Inferred finds none; docs/journal/automatic-
  postings.md:95-96 document only two cases. Ran a probe binary against the crate and hledger 1.52
  on four scratch journals under /tmp/v50-scratch.

- **L4** `crates/ledgeline-core/src/auto_edit.rs:102` (CONFIRMED)
  auto_edit::apply is LF-only: on a CRLF main journal Remove leaves a doubled blank line (and a
  trailing blank when the block ends the file), Replace/Append splice LF-terminated lines into the
  CRLF file (Append also inserts a bare \n after \r\n), and save_rule's read-back guard cannot catch
  it because both sides compare the parser's `raw`, which is built from text.lines() and already LF-
  normalized; the sibling projections writer (serialize.rs auto_splices/removal) is CRLF-aware and
  shares only block_spans.
  *Scenario:* CRLF main journal (hledger reads it fine) with two `=` blocks: PUT remove block 0 →
  `account revenue:sales\r\n\r\n\r\n= expenses:rent…` (two blank lines); replace block 1 → two bare-
  LF lines inside the CRLF file; add → bare \n after \r\n then an LF block; remove last block →
  trailing blank. Reparse yields LF-normalized raws == expected_raws, so after != wanted is false
  and the mixed-terminator text is written silently.
  *Evidence:* auto_edit.rs:96, 102-103, 111-116; parse.rs:550 lines(), 1711-1714 raw with \n;
  auto_api.rs:431, 457-462, 472-474; auto_wire.rs:161; serialize.rs:639, 672, 680, 702-704 CRLF-
  aware; byte-exact python re-implementation validated against the module's LF tests then run on the
  CRLF journal, readback after==wanted True for every edit; hledger 1.52 reads the mixed file (exit
  0).
  *Also (D1-5):* Adds the span.start == 0 (rule first in file) case, which serialize.rs:702-704
  handles and auto_edit does not.
  *Same mechanism reported as:* A1-6

- **A3-3** `crates/ledgeline-core/src/hledger_conf.rs:192` (CONFIRMED)
  hledger 1.52 accepts options on a section-heading line (`[balancesheet] --depth 1`, space or TAB,
  scoped to that section), but heading() only recognises a line that is entirely `[...]`, so every
  reader (parse, hijacks_command, scan/ConfSettings::read) tokenises that line into the PRECEDING
  section: as the first line it is a false hijack (Settings locked behind a red alert, every PUT
  400, all settings default, [import] aliases dropped from imports); after any general option it is
  read as a GENERAL depth (wrong effective depth for the other reports), and saves then edit or
  delete the option in the wrong scope or append a duplicate [balancesheet] section hledger reads
  instead.
  *Scenario:* File `[balancesheet] --depth 1`: hledger bs depth 1, bal unlimited (verified);
  Ledgeline: hijacked_by=Some('[balancesheet]'), editable=false, every PUT 400, in_force=false. File
  `--pretty\n[balancesheet] --depth 1`: general depth 1 shown; saving balancesheet depth 2 appends a
  second section; clearing the phantom general depth deletes the user's balancesheet-scoped depth
  with the read-back passing.
  *Evidence:* hledger_conf.rs:192-196, 182, 263, 250-256, ~1582, 1161-1164; conf_api.rs
  ConfState::read, target(), body(), run_put; import_api.rs:1425-1434; HledgerConfPanel.svelte:58,
  126-128; ran hledger 1.52 on three scratch confs (space, TAB, after --pretty) plus the post-save
  layouts; compiled verbatim heading/parse_tokens/hijacks_command with rustc; docs never say a
  heading must be alone on its line.

- **A3-6** `crates/ledgeline-core/src/hledger_conf.rs:301` (CONFIRMED)
  alias_arguments returns the --alias values of EVERY [import] heading (parse() tags options by
  section name only, no occurrence), whereas hledger 1.52 applies only the LAST duplicate heading's
  options (verified with --debug=2), so Ledgeline's import forwards aliases a terminal `hledger
  import` ignores and run_write_conf treats such an alias as 'already present' and writes nothing;
  byte-identical to main, but the branch added last-occurrence handling in this same module and its
  new module docs (67-70) describe last-only reading as the verified model without applying it here,
  and DuplicateSection warns only for the four DepthCommand headings, never [import].
  *Scenario:* hledger.conf with two [import] headings (Chase alias in the first, Expenses alias in
  the second): terminal `hledger import` writes Chase:Checking / spend:Food; Ledgeline's --no-conf
  invocation forwards both aliases and writes assets:chase:Checking — the two-journals divergence
  this module exists to prevent; the alias fix button computes nothing to add; no warning shown.
  *Evidence:* hledger_conf.rs:175-189, 293-308 (unchanged from main), 881-886, 1165-1172, 1211-1216,
  docs 67-70; import_api.rs:1425, 5729-5731 (context lines only in the diff); hledger 1.52 scratch
  runs with duplicate [import], duplicate [print], and a single merged section; existing test covers
  a single [import] only.

- **E-2** `crates/ledgeline-core/src/hledger_conf.rs:346` (CONFIRMED)
  locate_in silently skips a symlinked hledger.conf beside the journal whose target lies outside the
  vault root (canonicalize → OutsideRoot → .ok()? → keep walking), so the config in force for
  --auto/--depth/alias forwarding is a parent's, ~/.hledger.conf, or none, while hledger 1.52 reads
  the symlinked file ('using nearest config file'); the above-root and home/XDG tiers use
  fs::metadata, which follows symlinks, so the identical link is honoured one directory above the
  journal and ignored beside it. locate_in predates the branch but main's only production caller was
  import alias forwarding; this branch newly routes the main journal's --auto switch and the
  Settings/--depth path through it. (The candidate's create_new→409 consequence is refuted: target()
  refuses the symlink, create is never offered, PUT is a 400; the UI shows 'Config file: none' plus
  'Shown but not editable here: it is not a regular file (a symlink…)'.)
  *Scenario:* Desktop, ~/books/main.journal, ~/books/hledger.conf → ~/dotfiles/hledger.conf
  containing --auto (dotfile-manager layout): auto_setting returns default (in_force false),
  apply_journal skipped, every report omits the generated postings hledger adds; --depth defaults
  lost; Settings says no config AND that the file is a symlink that cannot be edited. Verified
  hledger 1.52 uses the symlinked file.
  *Evidence:* hledger_conf.rs:340-354 (doc 336-337), 454-455, 635-636 fs::metadata;
  vault/local.rs:163-169 OutsideRoot (test canonicalize_resolves_inside_and_refuses_outside);
  location.rs:75-77; lib.rs:267, 274-275; auto_api.rs:124; conf_api.rs:99, 321, 372-374, 455-457;
  HledgerConfPanel.svelte:118-131; conf_endpoints.rs:335-350 covers only a symlink ABOVE the root;
  docs/settings/hledger-config.md:39 documents read-only, not ignored.
  *Same mechanism reported as:* D1-3

- **SW-2** `crates/ledgeline-core/src/parse.rs:2167` (CONFIRMED)
  every_pair() turns `every N <singular unit>` (every 2 week, every 15 day, every 3 month, every 2
  quarter, every 2 year) into PeriodKind::Every, but hledger 1.52 refuses every one of those
  spellings and will not load the file at all, so a hand-written rule projects and budgets in
  Ledgeline with no warning while `hledger --forecast`/`print`/`bal` on the same file fail —
  contradicting the parser's own contract (parse.rs:1803-1806: forms hledger rejects must become
  Unsupported). The acceptance predates the branch (main's [count, unit] arm had it); the branch
  rewrote it into every_pair and excluded only the -ly adverb. The UI's period cell cannot produce
  the text; the trigger is a hand-edited file, which the UI directs users to for non-simple
  recurrences.
  *Scenario:* Scenario or main journal with `~ every 2 week / (a) $1`: Ledgeline enumerates a
  biweekly flow, shows `every 2 week` as the editable base and writes it back verbatim on a From/To
  edit; `hledger -f scenario print` exits 1 ('unexpected "week" expecting "days", "months", …'), so
  the documented `hledger -f main -f scenario --forecast` cannot read the file.
  *Evidence:* parse.rs:2095, 2109, 2166-2168, 2208-2216, 1803-1806; plans/21:76; plans/30:87-92;
  hledger 1.52 scratch runs for 12 spellings; area-core.diff:1008-1083 provenance; tests at
  parse.rs:4160, 4264, 4486 and the parity fixtures never use a singular; projections_api.rs:1166 no
  validity check; serialize.rs:857, 1103 write raw verbatim.

- **A5-4** `crates/ledgeline-core/src/projections.rs:519` (CONFIRMED)
  auto_rule_account_warnings builds `known` from Scenario::named_accounts() (lines/events/cash only)
  plus the MAIN journal's accounts, while the scenario file's own `account` directives are parsed
  into serialize.rs's private Read.own_accounts and dropped, so a scenario rule posting to an
  account the scenario file itself declares is reported as 'no account directive declares' — a false
  statement about the file; docs promise 'declare it with an account directive and the warning goes
  away'.
  *Scenario:* Open fixtures/projections-parity/main.journal and run fixtures/projections-
  parity/auto-form.journal (declares ten accounts on lines 15-24; rules post to five of them that
  main never names) → projection.warnings gets five spurious 'check the spelling' entries. No
  existing test exhibits it (parity tests never call the function; endpoint tests use scenarios with
  no account directives).
  *Evidence:* projections.rs:437-456 Scenario has no declared-accounts field, 486-505
  named_accounts, 519-521; serialize.rs:186 .map(|read| read.scenario), 190-196, 238-242,
  own_accounts read only at 835; projections_api.rs:1851, 1327-1330; real_side.rs:100-110;
  auto_form.rs:719-726; plan 30 amendment 15; docs/projections/file-format.md:286-289;
  docs/journal/automatic-postings.md:101.

- **A1-3** `crates/ledgeline-core/src/projections/forecast.rs:1042` (CONFIRMED)
  The projection path applies a scenario's `=` rules through apply_counted, which hands `tag:` terms
  no account-directive tags (own or inherited), so `= tag:taxable tag:_generated-transaction` never
  fires on a forecast posting to an account the SCENARIO FILE declares with `; taxable: yes` (or its
  children), while hledger 1.52 `--forecast --auto` fires it on the same file; projected cash/net-
  income/runway silently omit the rule's legs with no warning. (Correction: a tag declared only in a
  sibling -f main.journal is not inherited by hledger either, so the deviation is confined to
  declarations in the scenario file or a file it includes.) Also: posting-level tags on `~` postings
  are dropped (forecast.rs:1778 tags: Vec::new()).
  *Scenario:* Scenario: `account expenses:food ; taxable: yes` + `= tag:taxable tag:_generated-
  transaction / liabilities:tax *0.1 / expenses:tax-accrual *-0.1` + `~ monthly lunch budget /
  expenses:food:lunch $100 / assets:checking` → hledger prints each forecast with the two tax legs;
  Ledgeline's forecast has only the two written postings, generated=false, warnings [].
  *Evidence:* forecast.rs:1042 apply_counted; auto.rs:61-66 passes &|_| Vec::new() vs 143-145/171
  (apply_journal) and 197-200 (apply_in) passing inherited_account_tags; query.rs:165-173 three tag
  sources; Scenario has no account-declaration field; serialize.rs:235-241 drops directive tags;
  plans/30 amendment 19 records the gap but docs/journal/automatic-postings.md:43 and
  docs/projections/file-format.md:200-235 do not. Ran hledger 1.52 on four scratch configurations
  and a cargo probe against ledgeline-core; outputs as described. Parity harness
  projection_files.rs:1456-1475 runs the shape where hledger honours the scenario's own account
  tags.

- **A5-5** `crates/ledgeline-core/src/projections/forecast.rs:1067` (CONFIRMED)
  After the scenario's `=` rules run, every generated posting's commodity is folded into
  `commodities` with no filter, so a documented bare-number rule posting (`(x) 2`, parsed as
  Commodity('') when no D directive is in effect) is reported by warn_on_commodity_mismatch as an
  unpriced stranger and the Projections page shows the garbled warning 'the scenario is partly in ,
  which no price converts into $' on every run where a valuation target exists and the rule fires.
  *Scenario:* Journal with any P directive or @ cost (fixtures/sample.journal has 14), scenario with
  `~ monthly expenses:rent $4200` plus `= expenses:rent tag:_generated-transaction /
  (expenses:rent:fee) 2` → POST /run warnings[0] as quoted; with a `3 CHF` leg: 'partly in , CHF,
  which …'. `$2` removes it.
  *Evidence:* forecast.rs:1067-1073, 907-910, 541; auto.rs:519; parse.rs:2847-2858;
  projections.rs:1363-1381; reports/prices.rs:128, 729-770; docs/projections/file-format.md:233.
  Reproduced by running target/debug/ledgeline --server on a /tmp copy of sample.journal and POSTing
  /api/projections/run.

- **A5-3** `crates/ledgeline-core/src/projections/forecast.rs:1734` (CONFIRMED)
  The untagged-rule warning (and docs/projections/file-format.md:219-220 and the endpoint test
  pinning its text) misstates hledger 1.52: under the documented sibling command `hledger -f
  main.journal -f projection-x.journal`, hledger applies a scenario's `=` rules only to that file's
  own `~` forecasts and never to main's recorded transactions, tag or no tag — exactly what
  Ledgeline does — so the warning describes a divergence that does not exist and the tag it
  recommends changes nothing (plan 30 lines 143-147 say so; Decision 6 contradicts it). In the one
  configuration where hledger would reach recorded transactions (main `include`s the scenario), the
  server has already applied the rule to recorded transactions under --auto, so 'applies only to
  projected transactions here' is false there instead.
  *Scenario:* Scenario holding `= expenses:rent / (budget:rent) *-1` → Projections warns 'hledger
  --auto would also apply it to recorded ones; add tag:_generated-transaction'; running the
  documented check leaves every recorded transaction untouched, so the user edits the file for
  nothing or wrongly concludes hledger disagrees.
  *Evidence:* forecast.rs:1733-1736; hledger 1.52 on /tmp scratch: `-f main -f scen print --auto
  --forecast` → no expenses:tax on main's recorded/forecast wages, scen's own forecasts do get it;
  `include` variant → recorded txn gains the postings; lib.rs:274-275 apply_journal;
  auto.rs:118-119; parse.rs:181-218, 762; docs file-format.md:219-220, index.md:19; test
  projection_endpoints.rs:3055 asserts the wrong text.

- **L6** `crates/ledgeline-core/src/projections/real_side.rs:101` (CONFIRMED)
  Decision 11's 'known account' set is computed by two drifted definitions — auto::known_accounts
  (main journal: excludes generated postings, includes `~` periodic postings) vs JournalFacts.known
  (projections: every posting including generated, no periodic) — and because JournalFacts::new is
  built from the APPLIED snapshot journal under --auto, a scenario rule posting to a typo'd account
  that a firing main-journal rule also posts to is silently accepted, while the same scenario warns
  the moment --auto is off; conversely an account named only by a main `~` rule warns in a scenario
  but not in Settings.
  *Scenario:* --auto on; main journal `= expenses:payroll / expesnes:payroll-tax *0.1 /
  assets:checking` with a real expenses:payroll txn; the scenario carries the same rule for
  forecasts → POST /run: known contains expesnes:payroll-tax via the generated posting → no 'check
  the spelling' warning. Flip --auto off → identical scenario warns. Second drift: `~ monthly
  budget:food $500` only; main `=` rule posting to budget:food → Settings silent; same rule in a
  scenario → warns.
  *Evidence:* lib.rs:274-275 applied journal; projections_api.rs:1311,1317; projection_cache.rs:62;
  real_side.rs:101-107 no generated filter, no periodic; projections.rs:519-521;
  auto_form.rs:703-713; auto.rs:407-424 (417 generated filter, 419-422 periodic);
  auto_journal.rs:366-402 pins the main half ignoring generated. hledger 1.52 scratch: `accounts`
  omits both, `accounts --auto` lists the typo, `accounts --forecast` lists budget:food. No test
  covers --auto on + shared account (projection_endpoints.rs:2991 runs without --auto).
  *Also (S2):* Same mechanism as cleanup: two implementations of the unknown-account check with
  drifted '%' handling ('%' vs '%account'), known-set semantics and wording ('check the spelling' vs
  'Is it a typo?').
  *Also (R1):* Hand-rolled ancestor expansion in auto::known_accounts duplicates
  AccountName::self_and_ancestors / wire::journal_to_accountnames (not a drop-in: different posting
  filters); auto.rs:334 capitalized is byte-identical to account_groups.rs:455.

- **A5-2** `crates/ledgeline-core/src/projections/serialize.rs:786` (CONFIRMED)
  commodity_declarations builds `used` from scenario.lines (amount + opening) and scenario.events
  only, never from the `=` rules' AutoAmount::Fixed / Scale{commodity} amounts, so a rule whose
  fixed amount is in a commodity no `~` line or event uses gets no `commodity` directive on save and
  `hledger -f main -f scenario --auto --forecast` reads a hand-typed `€1,500` as €1.500 while
  Ledgeline (inherited style) reads 1500 — the drift the function exists to stop; narrow trigger
  since the form writer never emits digit groups.
  *Scenario:* Main declares `commodity €1,000.00` and `$1,000.00`; scenario has a $ rent line and a
  hand-typed rule `= expenses:rent / expenses:fee €1,500 / assets:checking`; after save hledger
  prints `expenses:fee €1.500` and `bal expenses:fee` = €3.00 for two firings (Ledgeline: €3,000);
  adding the directive fixes it.
  *Evidence:* serialize.rs:786-797 no auto_rules reference; parse.rs:755-760, 2859-2865 rule amounts
  read under inherited styles; auto_form.rs:364-384 digit_groups None; hledger 1.52 scratch run
  under /tmp/v39 as quoted; docs/projections/file-format.md:189-194 promise the opposite.
  *Same mechanism reported as:* C-3

- **A4-5** `crates/ledgeline-core/src/query.rs:165` (CONFIRMED)
  `tag:payee=X` / `tag:note=X` are, in hledger 1.52, the SAME query as `payee:X` / `note:X` (matched
  against the description's payee/note, literal tags ignored); Ledgeline compiles them to a plain
  Term::Tag over literal+inherited tags, so the rule silently never fires on an ordinary transaction
  and DOES fire on one carrying a literal `payee:`/`note:` comment tag whose value matches,
  generating postings hledger would not.
  *Scenario:* `= tag:payee=acme / (auto:hit) 1` over `2026-01-01 * (42) acme | team lunch`: hledger
  adds the posting under both legs, Ledgeline nothing and no diagnostic; `= tag:payee=bob` over
  `dinner ; payee: bob`: Ledgeline generates two postings, hledger none.
  *Evidence:* query.rs:300-303 (no payee/note branch), 165-173; hledger-lib 1.52 Query.hs
  matchesPosting (Tag n v) special-cases 'payee'/'note'; ran hledger 1.52 print --auto on four
  scratch journals and the real crate via a throwaway binary — outputs as quoted;
  docs/journal/automatic-postings.md:43 and query.rs:14-17 describe tag: purely as a tag match.

- **A4-3** `crates/ledgeline-core/src/query.rs:259` (CONFIRMED)
  tokens() opens a quote on any `'` or `"` wherever it appears in a term, so a single apostrophe
  inside a term (desc:O'Brien, desc:Joe's, acct:Bob's) makes the whole query unsupported ('a quote
  in the query is never closed') and the rule is locked and never fires, whereas hledger 1.52 reads
  a mid-term apostrophe literally and fires; a balanced pair inside one term (desc:O'Brien's) is
  worse: silently tokenised to desc:OBriens, so Ledgeline evaluates a different regex from hledger
  with no lock shown. Everyday payees (Trader Joe's, Lowe's, Macy's) make it common.
  *Scenario:* --auto with `= desc:O'Brien / (assets:flag) 1` and txn 'O'Brien plumbing': hledger
  adds the flag below both postings; Ledgeline skips the rule, shows the lock, form path refuses any
  extra query with an apostrophe. `= desc:O'Brien's` → Ok(['desc:OBriens']): not locked, fires on
  'OBriens plumbing' (hledger does not) and not on 'O'Brien's plumbing' (hledger does).
  *Evidence:* query.rs:250-282 (259-261 no start-of-term check, 275-276); ran the function verbatim
  with rustc; ran hledger 1.52 print --auto on scratch journals; parse.rs:1685, 1717; auto.rs:84;
  auto_form.rs:441, 514; AutoRulesPanel.svelte:270; tests query.rs:481-497 cover only a leading open
  quote; docs name only expr: as a lock cause.

- **A4-6** `crates/ledgeline-core/src/query.rs:354` (CONFIRMED)
  Query patterns are handed to Rust `regex` verbatim, so hledger's regex-tdfa dialect diverges both
  ways: a `{` not followed by a digit is a literal in hledger (`= expenses:{misc}` parses and fires)
  but a parse error here, which locks the rule (loud, covered by the lock contract); `\d` is a
  literal `d` in hledger but a digit class here (Cargo.toml enables unicode-perl), so `= acct:\d`
  silently fires on different postings in the two tools with no warning. Plan 30 asserts the
  dialects 'differ only at the edges', which is false for \d/\w/\s and a bare `{`.
  *Scenario:* --auto with `= expenses:{misc} / (budget:brace) 1` and `= acct:\d / (budget:digit) 1`
  over postings to expenses:{misc}, assets:401k, expenses:food: hledger adds brace+digit to the
  {misc} txn and digit to the food txn, nothing to 401k; Ledgeline locks rule 1 and fires rule 2 on
  assets:401k only.
  *Evidence:* query.rs:294, 327, 353-358; Cargo.toml:39 unicode-perl; hledger 1.52 scratch run as
  quoted; scratch crate pinned to regex 1.13.1: `expenses:{misc}` → parse error, `\d` matches 401k
  only; auto.rs:84; auto_form.rs:514; AutoRulesPanel.svelte:270; docs/journal/automatic-
  postings.md:64 covers the lock half only; plans/30:205-207; no test exercises \d or {.

- **A7-1** `crates/ledgeline-core/src/reports/account_report.rs:242` (CONFIRMED)
  In a balance section (Assets/Liabilities), an account that moved inside the window but whose
  closing balance is zero at every period end is dropped by shown() (cumulative values all zero
  after drop_zeros), yet its change is still counted in the parent row's Δ and the section summary
  (sum of unclamped change_totals) — a Δ no visible row explains; when it was the section's only
  account the whole section vanishes despite the movement. Docs say Show empty adds only 'accounts
  nothing touches in these periods', so the default hiding of a touched account is contrary to the
  stated rule.
  *Scenario:* Opening liabilities:card $-500, liabilities:loan $-10000; 2026-06-10 pay off card
  $500; Accounts report Jun–Jul monthly, Show empty off → rows `liabilities` Δ $500 over
  `liabilities:loan` Δ 0 with no card row; without the loan the Liabilities section is absent
  entirely while Assets shows checking Δ -500. Reproduced by running the real account_report via a
  scratch crate.
  *Evidence:* account_report.rs:222-227, 239, 242, 251-260; account_periods.rs:148-154, 172-180;
  reports_api.rs:3332-3342; real_side.rs:679; accountReportRows.ts:57; hledger 1.52: `bal -M -H`
  hides the row and prints no row-total column, `bal -M` lists card $500 — the two commands the docs
  cite disagree; docs/projections/index.md:84-91.

- **A7-2** `crates/ledgeline-core/src/reports/income_statement.rs:867` (CONFIRMED)
  group_rows names rows from the group's members only (clipped when a depth is given), so an
  ancestor with no direct posting in any reported window gets no row at depth None or any depth
  deeper than it even when its children do; `hledger is --tree` prints that parent line (verified),
  and in Ledgeline the row comes and goes with whether the ancestor happened to post in a reported
  window; the test pins the omission, and displayRows.ts's contract ('engine rows include every
  ancestor') is violated so children render as flat roots.
  *Scenario:* Wages fixture: range Mar 2026 (or any interval mode, or Projections at depth ≥ 4) → no
  `wages-salaries` row; george and john render as roots with full-name labels while hledger prints
  people-costs $1150 / wages-salaries $1100 / george / john. Hybrid: members a:b and a:b:c:d with no
  a:b:c posting → a:b:c:d emitted as a root sibling of a:b whose subtree already contains it (visual
  double count). Totals unaffected.
  *Evidence:* income_statement.rs:867-873, 1131-1136, doc 849-850, 262-266; displayRows.ts:2, 20-21,
  33-39; tests/income_statement_grouped.rs:1684-1700, 1802-1813 (pins the omission); hledger 1.52
  scratch run of `is --tree` prints the parent at depth 9 and 3.

### Server (crates/ledgeline-server)

- **A2-4** `crates/ledgeline-server/src/auto_api.rs:336` (CONFIRMED)
  Files::main() returns the first WRITABLE entry of journals::targets_in's import ranking
  (transaction-bearing files first, newest last_txn_date first), not the is_root target, so with an
  include-only main.journal `newRule` names 2026/2026.journal and '+ Add a rule' appends there —
  contradicting the wire contract (:150-151, types.ts:58), the struct comment (:297 'the main file
  first') and docs line 85 ('end of your main journal file'); and because rules apply in file order
  with an include's rules at the include line's position, a rule the user believes was appended to
  the end of main may precede (and never see the output of) rules written later in main.
  *Scenario:* Journal laid out like fixtures/import/layouts/split-year (main.journal = four include
  lines): GET /api/auto-rules → newRule.journalId = '2026/2026.journal'; PUT add → 200, block
  written to the end of 2026/2026.journal, main.journal byte-identical. A root whose own
  transactions are merely older than an include's gives the same. The existing test passes only
  because the fixture's main.journal holds every transaction.
  *Evidence:* auto_api.rs:308-311, 336, 262, 297, 150-152; journals.rs:136 is_root unused here,
  145-149 sort; core tests/journals.rs:84-97, 340-360 pin the ranking; budget_api.rs:1399,
  account_api.rs:690, prices_api.rs:626 use .find(|t| t.is_root). Live run of target/debug/ledgeline
  --server on a scratch split-year copy: GET and PUT outputs as described. hledger 1.52 chained-rule
  scratch runs confirm the order consequence. Mitigation: the button names the file it will write.
  The symlinked-root trigger (E-5) is unreachable: the root is canonicalized before opening.
  *Same mechanism reported as:* E-5, A1-2, D1-2

- **A1-5** `crates/ledgeline-server/src/auto_api.rs:488` (PLAUSIBLE, unreproduced)
  After the rule file is written, a failed reopen_editor() (which unbinds the editor WITHOUT
  publishing a new snapshot) is only logged and save_rule still answers 200 with listing(state)
  built from the PRE-write snapshot — old rule texts/forms/blocks and editable:false paired with
  POST-write revisions (Files::new re-reads disk) — so the UI shows the edit as not applied and
  read-only while the file already carries it; the generation is unchanged so the client's re-read
  returns the same stale listing and any retry is a 501. Trigger needs an environmental failure
  between the write and the re-open (evicted include, external write breaking the parse, symlink
  swap, or a concurrent watcher reload returning None which the `if let Some(Err)` skips); not
  constructible from user input alone. budget_api.rs:965-975 avoids this by building the response
  from new_text.
  *Scenario:* PUT replace succeeds; editor.reopen() fails → 200 body with rules[0].raw = pre-edit
  block, editable:false, revision = fingerprint of the NEW bytes; store ignores the body, generation
  unchanged, GET serves the same; retry → 501 editing-disabled. Confirmable with an endpoint test
  whose vault fails read_pinned/canonicalize once on the re-open.
  *Evidence:* auto_api.rs:485-488, 209, 215, 314, 319, 390-392, contract at 48; lib.rs:607-611
  (store only in Ok arm), 624-627, 762-776, 820-822, 845-850; budget_api.rs:965-975;
  store.svelte.ts:41-45; AutoRulesPanel.svelte:64; error.rs:110; no test of the reopen-failure path.

- **D1-4** `crates/ledgeline-server/src/conf_api.rs:461` (CONFIRMED)
  When the locator found nothing (conf.text '', revision '', located None) but a regular file exists
  at <root>/hledger.conf by the time target() runs read_pinned, the Ok arm reports exists:true
  without consulting conf.located, so run_put takes the replace branch with base = '' and overwrites
  the newly created file with just the requested option line (`--auto\n`, no header), pinned to that
  file's own revision; same two-read window as E-4, consequence is loss of a whole freshly created
  file. No non-race route to this arm exists.
  *Scenario:* No conf anywhere; Settings holds revision ''; PUT {revision:'', auto:true}; between
  locate_in's stat and target()'s read_pinned someone creates <journal dir>/hledger.conf with
  content C (hand-written aliases, git checkout, `hledger setup`) → '' == '' passes → read_pinned Ok
  → base = '' → next = `--auto\n` → replace passes → C replaced wholesale, 200.
  *Evidence:* conf_api.rs:98-102, 78-82, 446, 270-272, 309-312 (only the Err arm consults
  conf.located), 461-465, 473-478; hledger_conf.rs:1510-1543 insert_line on '' yields the option
  line only; 901-903 header only for the create branch. Non-race routes ruled out by reading
  locate_in's cases against target()'s refusal arms.

- **A3-7** `crates/ledgeline-server/src/conf_api.rs:462` (CONFIRMED)
  A hledger.conf that hledger 1.52 refuses outright (a Latin-1 byte such as 0xE9 anywhere in it, or
  a section heading with leading whitespace) is reported by GET /api/hledger-conf as in force,
  editable and warning-free, and its --auto/--depth are honoured by Ledgeline; a PUT that changes
  any option writes back the lossily decoded text (run_put's base is conf.text, read via
  from_utf8_lossy), so every invalid byte becomes EF BF BD on disk in addition to the requested
  change — contrary to the documented 'leave every other byte alone' — and the revision is a
  fingerprint of the lossy text so the 409 check never blocks it; the indented heading is left as
  is, so hledger still refuses the saved file and Ledgeline still shows no warning.
  *Scenario:* File `# caf\xE9\n--auto\n`: GET shows editable, auto:true, warnings []; `hledger
  print` exits 1 ('cannot decode byte sequence starting from 233'); user turns --auto off → file
  rewritten as `# caf\xEF\xBF\xBD\n`. Same substitution inside option values (`--depth
  assets:caf\xE9=2`). `--auto\n  [balancesheet]\n--depth 2` → hledger refuses ('unexpected [
  expecting # or newline'), Ledgeline honours depth 2 silently.
  *Evidence:* hledger_conf.rs:384, 413, 576-580, 1582 heading(body.trim()), 192-196, 1080-1093
  ConfWarning variants; conf_api.rs:105, 311, 436-477; vault/local.rs:192-200; docs
  hledger_conf.rs:91-92, 1387-1390, conf_api.rs:32-33, docs/settings/hledger-config.md:33. Ran
  hledger 1.52 on scratch dirs (refused under en_US.UTF-8, LC_ALL=C, ISO8859-1; indented headings
  refused; trailing spaces accepted) and a rustc program printing the lossy bytes.

- **E-4** `crates/ledgeline-server/src/conf_api.rs:477` (CONFIRMED)
  run_put derives `next` from the text of read #1 (ConfState::read, whose pinned Revision read_in
  discards; the Native tier has no pin at all) but pins the atomic replace to the Revision of read
  #2 (target() → read_pinned, whose bytes are discarded), so an external write landing between the
  two reads passes the stale check (against read #1's fingerprint) and the pinned replace (against
  read #2's identity, which the disk still equals) and is overwritten by an edit of the older text
  with a 200; auto_api::save_rule reads once and pins from that same read.
  *Scenario:* hledger.conf = `--depth 2` (T1); PUT {revision: fp(T1), auto: true}; between line 441
  and 454 an editor/dotfile sync writes T2 = `--depth 2\n--alias a=b`; 446 passes; 309-311 pins P2;
  next = edited(T1) = `--depth 2\n--auto`; 477 replace(next, P2) passes → `--alias a=b` silently
  lost, 200, no 409. The import_writes lock serialises only Ledgeline's own writers.
  *Evidence:* conf_api.rs:441, 103-105, 446, 454, 309-311, 461-466, 475-477; hledger_conf.rs:383-384
  read_in discards Revision, 401-404 Native read unpinned; vault/mod.rs:168-170 matches();
  pinned.rs:543-544; contrast auto_api.rs:422-423, 474 (one read); tests conf_endpoints.rs:252-260,
  320-331 never exercise a write between the reads.

- **SW-1** `crates/ledgeline-server/src/import_api.rs:3392` (CONFIRMED)
  reconcile_ids indexes snapshot.journal, which under --auto is the APPLIED journal, so every
  previously imported transaction a `=` rule fires on carries extra generated postings and
  reimport::diff() classifies it Conflicting ('postings: 3 vs 2') instead of Unchanged/StatusOnly —
  its status flip is dropped (the commit acts only on the preview's flips) and the UI reports a
  conflict the server itself manufactured; with a broad rule that is nearly every imported row,
  disabling status sync entirely under the new flag. Rows are not duplicated and nothing wrong is
  written. Same mechanism in qb_journal_api.rs:372/1038 (guarded on write). Fix: index snapshot.raw.
  *Scenario:* --auto on; `= expenses:food / (budget:food) *-1`; previously imported `2026-01-05 !
  COFFEE SHOP ; id:FIT0001`; the bank re-download shows it cleared → preview: conflicting [{FIT0001,
  diffs: [status pending→cleared, postings 3→2]}] 'left untouched'; commit: the row stays `!`
  forever and every later re-download repeats the bogus conflict.
  *Evidence:* lib.rs:148-155, 274-278; import_api.rs:3391-3392 (unchanged by the branch — on main
  journal==raw always), 3470-3475, 3491; reimport.rs:82-88, 249-259, 326-329, 420-425;
  edit_api.rs:756-758; importModel.ts:609-611; probe crate: RAW → StatusOnly, APPLIED → Conflicting
  with the quoted diffs; hledger 1.52 confirms the rule is legal; no endpoint test covers re-import
  under --auto.

- **E-1** `crates/ledgeline-server/src/lib.rs:714` (CONFIRMED)
  refresh_conf's None arm seeds an empty conf slot with the post-change reading and never compares
  it to the `auto` the snapshot was built from (lib.rs:267), so on a watcher-less host (FFI mobile
  builds with default-features=false; its only freshness path is state.refresh()) an external --auto
  toggle that lands while the slot is empty — the state after EVERY publish, since only the desktop
  watcher's watched_files() fills it — is swallowed: refresh() reports Unchanged, GET /api/auto-
  rules says not in force and reports serve the un-applied journal while GET /api/hledger-conf
  answers auto:true, stuck until a journal edit or a FURTHER conf change. Desktop is masked only
  because the watcher's reload() rebuilds unconditionally.
  *Scenario:* Watcher-less host; snapshot published with no conf; user/iCloud writes --auto into
  hledger.conf; refresh() → Ok(Unchanged), generation unchanged, /api/auto-rules inForce=false, the
  sale served with 2 postings, /api/hledger-conf auto=true, second refresh() Unchanged again.
  Control with the slot pre-filled: refresh() → Reloaded, inForce=true, 4 postings.
  *Evidence:* lib.rs:704-716 None arm; 267 separate eager read; 274-278; 797-806 refresh();
  edit.rs:532-544 changed_sources ignores hledger.conf; .conf() callers only 694 (test), 711, 746
  (watcher); republish() only called from tests; conf_endpoints.rs:320-331 gets BEFORE the write so
  only exercises the Some arm. Reproduced with a /tmp probe linked against the crate
  (router_with_state), three runs as described.
  *Also (S1):* AutoSetting is field-for-field derivable from ConfState; duplicate test
  auto_api.rs:583-594 vs hledger_conf.rs:2070-2078; republish_with_conf re-reads disk then
  overwrites the slot.
  *Also (E4):* 1-2 redundant locate+read per snapshot (three on a conf PUT); small beside the
  snapshot build.
  *Same mechanism reported as:* L1, R4

### Web (web/src)

- **D2-5** `web/src/lib/autoRules/AutoRuleEditor.svelte:80` (CONFIRMED)
  The '% of matched' box parses with parseAmountInput, which strips every comma before matching, so
  a decimal-comma entry '3,5' is accepted as 35 (0,5 → 5; 12,50 → 1250) with no invalid flag; the
  box re-renders as 35 and Save writes `*0.35` instead of `*0.035`.
  *Scenario:* Type `3,5` in '% of matched' (inputmode=decimal; comma-decimal mobile keypads offer
  only ','), tab out → parseAmountInput('3,5') → {m:35,p:0} → posting.percent=35 → Save → factor_of
  writes *0.35 (35% of the matched amount).
  *Evidence:* editMapping.ts:62 replace(/[\s,]/g,'') then :64 regex; AutoRuleEditor.svelte:80-87
  only flags parsed===null; auto_form.rs:213-216 factor_of; ran in node: '3,5'→35n, '0,5'→5n,
  '12,50'→1250n.

- **D2-7** `web/src/lib/autoRules/AutoRuleEditor.svelte:110` (CONFIRMED)
  dropPosting resets the per-posting `invalid` and `pendingCommodity` maps wholesale instead of
  deleting key `at` and shifting higher keys, so removing ANY posting silently discards every other
  row's unreadable number text (box reverts to the last good value, error flag clears) and every
  commodity typed before its number (box reverts to the default commodity, and the next number typed
  is stored in that default).
  *Scenario:* 3 postings; posting 1 has '12.5.3' typed (flagged); posting 2 is fixed, amountless,
  with 'EUR' typed in its commodity box; click Remove on posting 3 → posting 1 reads '3' unflagged,
  posting 2's commodity reads '$'; typing 25 then stores {commodity:'$', quantity:25}.
  *Evidence:* AutoRuleEditor.svelte:108-113 `invalid = {}; pendingCommodity = {};`; readers 116, 65;
  187 index-keyed each keeps rows mounted; neither host remounts (AutoRulesPanel.svelte:156-168;
  ProjectionsTable.svelte:617-620). Reproduced with a scratch vitest under /tmp: AFTER REMOVE
  posting1.value='3' aria-invalid=false posting2.commodity='$'.

- **A2-2** `web/src/lib/autoRules/AutoRuleEditor.svelte:135` (CONFIRMED)
  A hand-written un-anchored rule (`= revenue:sales`) is read into the form and labelled 'or one of
  its subaccounts', but as written it fires on ANY account containing that text (revenue:sales2,
  revenue:sales-tax, archive:revenue:sales — verified in hledger 1.52 and the engine's infix regex),
  and every form commit — including a blur of the match field with no change in Projections (which
  also sets rule.edited), or Settings Edit→Save with nothing changed — re-renders it as
  `^revenue:sales(:|$)` so those firings silently stop.
  *Scenario:* Scenario holds `= revenue:sales / expenses:commission *-0.03 / assets:checking` and
  posts to revenue:sales and revenue:sales-tax; user clicks into 'Matched account' and tabs away →
  AccountInput.onBlur → onCommit → rule.edited=true → next Save sends {form} → engine renders `=
  ^revenue:sales(:|$) tag:_generated-transaction` → revenue:sales-tax commissions vanish from the
  projection and from `hledger --forecast --auto`, no warning. Settings Edit→Save unchanged does the
  same to the main journal.
  *Evidence:* AutoRuleEditor.svelte:132 onCommit={onChange}, :135 label; auto_form.rs:645-655
  account_term accepts a plain body, :412-419 always writes ^…(:|$); test :975-995 asserts the
  rewrite; query.rs:352-359 infix; hledger 1.52 scratch: `= revenue:sales` fires on
  sales2/archive:revenue:sales; AccountInput.svelte:222-226 no change check;
  ProjectionsTable.svelte:617-619, 1317; scenarioModel.ts:916, 922-923; AutoRulesPanel.svelte:89,
  120 no comparison; auto_api.rs:451-454 no-op guard only byte-identical. Docs say a hand-written
  rule is saved in form shape once edited, not that a blur or unchanged Save counts.

- **A2-5** `web/src/lib/autoRules/decode.ts:27` (CONFIRMED)
  nullableStr reads an ABSENT key as null, so a missing journalId, revision, unsupported, conf or
  problem.account makes decodeAutoRules return a well-formed listing instead of throwing
  ApiShapeError as the module header promises and as the sibling decodeNullableStr
  (nativeDecode.ts:1547-1552) does for the same `unsupported` field — a latent drift-guard gap that
  fires only under a Rust field rename or web/server version skew (the server today always emits the
  keys as null).
  *Scenario:* GET /api/auto-rules whose rules[i] lacks journalId (after a rename) → journalId null →
  canEdit false for every rule → no Edit/Delete and no error. `unsupported` absent → lock badge and
  'never fires here' note vanish and an unsupported rule shows 'N generated postings'. `conf` absent
  → 'no hledger.conf is in force' although one is.
  *Evidence:* decode.ts:27, 50, 62, 65, 67, 82; header 2-3; nativeDecode.ts:1549 throws on undefined
  with rationale (~1541-1546, 2386-2389), 2532 strict for the same field; auto_api.rs:143-197 plain
  Option<String>, no skip_serializing_if; AutoRulesPanel.svelte:64, 231, 269-273; node -e:
  nullableStr(absent) → null, decodeNullableStr(absent) → throws; decode.test.ts never deletes a
  nullable key.

- **A2-7** `web/src/lib/autoRules/ui/AutoRulesPanel.svelte:119` (CONFIRMED)
  An existing rule's open editor is rendered on `editing.key === rule.index` alone (line 308), never
  re-checked against canEdit after the listing is re-read, so when the re-read rule N carries
  journalId/revision null (file no longer a writable target, read failure, or no storage) the editor
  stays open and Save returns silently at line 119 with no message — a dead Save button. The
  candidate's Delete and new-rule halves are unreachable from the UI (confirm button sits inside the
  canEdit guard; the new-rule editor is wrapped in `loaded.editable && loaded.newRule !== null`).
  *Scenario:* Edit rule N; a listing reload lands under the open editor (poller sees a new
  generation, or a 409 reload) with rules[N].journalId null → keyed each retains block N, editing
  survives, form + Save stay on screen → Save → `if (rule.journalId === null || rule.revision ===
  null) return;` → no PUT, no editing.error, button still enabled.
  *Evidence:* AutoRulesPanel.svelte:308-309 no canEdit; :278 guard wraps only the buttons; :63-65;
  :119 vs :110-113 (which sets editing.error); :183; :240-242; :45-49 reload effect;
  store.svelte.ts:49; auto_api.rs:282, 284, 300-325, 410-414. editable=false is NOT silent (server
  editing_disabled lands in editing.error). No test covers a listing change under an open editor.

- **A2-6** `web/src/lib/autoRules/ui/AutoRulesPanel.svelte:123` (CONFIRMED)
  While a PUT is in flight only Save, Cancel and confirm-Delete carry disabled={saving}; Edit/Delete
  on other cards, '+ Add a rule' and 'Edit as text' stay enabled and startEdit/startAdd replace
  `editing` without checking `saving`, so save()'s post-await code (which checks only `editing ===
  null`, never the key) closes or writes its error into whichever editor is open when the request
  returns. The right block is still written; the damage is the lost draft and the misattributed
  error/closed editor.
  *Scenario:* Edit rule 1 (10→8), Save (in flight); click Edit on rule 2 → rule 1's draft vanishes,
  rule 2's editor opens; PUT 409 → 'The file changed … make the change again' renders inside rule
  2's card; PUT 200 → `editing = null` closes the editor the user just opened (a '+ Add' form opened
  mid-save loses its typed content).
  *Evidence:* AutoRulesPanel.svelte:122-125, 67-76, disabled={saving} only at 183/186/285, Edit 291,
  Delete 294-299, Add 245, 'Edit as text' 189-194, 278. Reproduced with a scratch vitest under /tmp
  (2/2 passed): pending PUT, Edit on rule 2 enabled and switching, 409 error lands in rules[1], 200
  closes the new-rule editor. No existing test interacts during an in-flight save.

- **A6-5** `web/src/lib/projections/export.ts:118` (CONFIRMED)
  The Runway download's filename spans the whole statements window (first lookback period start →
  last forecast period end) and the Runway chart now draws those actual months, but the sheet holds
  only the opening balance and the forecast buckets (cash.history is dropped) and labels rows with
  raw bucket keys (`2026-08`, `2026-Q3`), so the filename names periods the file does not contain
  and chart and download disagree about what the reading covers.
  *Scenario:* Default Actuals=2, monthly, cutoff 2026-08-01, 2 periods: chart shows Jun, Jul
  (actual), Aug, Sep (projected); Export downloads
  `projection-<slug>-runway-2026-06-01-to-2026-09-30.xlsx` whose rows are 'Opening balance
  (actual)', '2026-08', '2026-09' — nothing dated June or July, labels differ from the chart axis
  and from every other sheet.
  *Evidence:* export.ts:117-119 span from balanceSheet.columns used for every tab; 138-145 runway
  built from buckets/opening/cash.values (no history); xlsx.ts:873-876 raw bucket keys;
  export.test.ts:50, 90-92 vs ProjectionReports.svelte.test.ts:74 show both halves on one fixture;
  docs/projections/index.md:61-63 written before c5ebe50 which touched neither export nor docs.

- **D2-2** `web/src/lib/projections/export.ts:121` (CONFIRMED)
  The workbook title built from the free-text scenario name is also used as the exceljs worksheet
  NAME by every projection builder except the Journal's, so Export throws for any scenario name
  containing `* ? : \ / [ ]` or a leading/trailing `'`, and is silently truncated to 31 chars
  otherwise (prefixes are 19-29 chars).
  *Scenario:* Scenario saved as 'Plan A/B' or '2027: lean year' (Save-as has no constraint;
  serializer refuses only commas) → Projections → Export .xlsx on Runway/BS/IS/CF/Accounts →
  workbook.addWorksheet('Projected Runway — Plan A/B') throws → toast 'Export failed: Worksheet name
  ... cannot include...'; no file. 'Base case' → IS sheet silently named 'Projected Income Statement
  — Ba'.
  *Evidence:* export.ts:120-121, 127; xlsx.ts:679, 714, 753, 868, 895 addWorksheet(meta.title);
  journalXlsx.ts:183 hard-codes 'Journal'; exceljs 4.4.0 worksheet.js name setter; reproduced with
  node -e in web/ (throws for '/', ':', '?', '[', ']'; truncates 'Patrick's plan');
  SaveScenarioDialog.svelte:79-88 unconstrained; serialize.rs:1538 refuses only ','; export.test.ts
  never asserts ws.name.
  *Same mechanism reported as:* A6-1

- **A6-6** `web/src/lib/projections/scenarioModel.ts:942` (CONFIRMED)
  ruleSample matches the rule's account against scenario line/event accounts case-sensitively
  (`===`/startsWith), while the rendered query `^ACCOUNT(:|$)` is compiled case-insensitively by the
  engine and by hledger, so the editor's preview falls back to its no-sample form exactly for a rule
  whose account differs only in case from the lines it fires on (preview only; no wrong money).
  *Scenario:* Flow line to `Revenue:Sales:EU` $1,000; rule matchAccount `revenue:sales` 5% → engine
  and hledger fire (verified) but ruleSample returns null → preview reads '≈ 5% to
  expenses:commission of each posting to revenue:sales' instead of '≈ −$50 … on each $1,000'.
  *Evidence:* scenarioModel.ts:939-942 no case folding; auto_form.rs:405-425 preserves case;
  query.rs:352-355 case_insensitive(true); ruleForm.ts:165-178; ProjectionsTable.svelte:1315;
  hledger 1.52 scratch fired on Revenue:Sales:EU; node -e: under('Revenue:Sales:EU') false.

- **D2-3** `web/src/lib/projections/ui/ProjectionsTable.svelte:1277` (CONFIRMED)
  The scenario rule list is keyed by its loop index, which Svelte 5 compiles as an UNKEYED
  positional block, so after Move up/down or Delete the surviving AutoRuleEditor instances stay at
  their positions and are re-pointed at a different rule's form while keeping their private $state
  (invalid text + error ring, pendingCommodity, open query-terms panel); a pending commodity typed
  on rule A can become the commodity of the next number typed on rule B and be saved.
  *Scenario:* Rules [A,B]; type 'abc' in A's percent box and blur (invalid[0]='abc'); Move down →
  [B,A]; block 0 now renders B.form but numberText returns invalid[0] → B shows 'abc' with input-
  error. Variant: type 'EUR' in A's amountless fixed posting's commodity box, delete A → B's same-
  index posting reads 'EUR' and the next number typed is committed as EUR via commodityOf and saved
  through onChange.
  *Evidence:* ProjectionsTable.svelte:1277 `{#each scenario.autoRules as rule, at (at)}`, 628-641
  splice; AutoRuleEditor.svelte:56,60,62,65,82,116, resets only in dropPosting 110-111, no $effect
  on form; compiled with svelte 5.57.2 → `$.each(ul, 21, ..., $.index, ...)`; compiler
  EachBlock.js:27-29 treats (i)-keyed as indexed; runtime each.js:316-320 re-points surviving
  positions. Other lists keyed by id (939, 1013, 1134). AutoRule has no id; block is null for new
  rules.
  *Same mechanism reported as:* A6-2

- **A6-4** `web/src/lib/projections/ui/ProjectionsTable.svelte:1290` (CONFIRMED)
  For an unedited hand-written scenario rule without tag:_generated-transaction, the host renders
  the 'projections only here' badge plus the 'Limit to forecasts in the file' button, while the
  AutoRuleEditor mounted directly beneath (default scope 'forecast') unconditionally renders a
  'forecasts only' badge whose tooltip asserts the rule IS written with the tag, ignoring
  form.forecastOnly === false — two contradictory statements on one card.
  *Scenario:* Scenario holds `= income:salary / expenses:taxes *-0.03` (no tag) → form served with
  forecastOnly=false → line 1290 shows 'projections only here' + button →
  AutoRuleEditor.svelte:150-155 shows 'forecasts only' with the 'is written with tag:_generated-
  transaction' tooltip.
  *Evidence:* ProjectionsTable.svelte:1290, 1294, 1297, 1310-1318 (no scope prop);
  AutoRuleEditor.svelte:36 default scope, 136/150-155 badge without forecastOnly check;
  auto_form.rs:517, 630; auto_wire.rs:89; test ProjectionsTable.svelte.test.ts:1253-1262 mounts this
  state but never asserts the badge is absent.

- **L3** `web/src/lib/reports/ui/confDepth.ts:29` (CONFIRMED)
  Per-account `--depth REGEX=N` is resolved in the browser with JS RegExp, a third engine beside the
  server's regex crate (validation) and hledger's POSIX ERE, and the three disagree: `[[:alpha:]]`
  is accepted by hledger and the regex crate but JS reads it as a bracket class and `search` returns
  -1 WITHOUT throwing, so depthLimit silently falls back to spec.all and a group hledger rolls up
  starts EXPANDED; in the other direction the server's regex_refusal accepts `(?i)inv` and `a**`,
  which hledger refuses, so a PUT writes a depth that makes `hledger bs` exit 1 ('Unable to parse
  depth specification').
  *Scenario:* Settings › hledger config › Balance Sheet: override `assets:[[:alpha:]]+:inv` depth 2,
  Save → both validators pass, written; `hledger bs` clips assets:brokerage:inv:* to one line
  (verified), but the Balance Sheet page's depthLimit returns Infinity → Brokerage group starts
  expanded. PUT perRegex [{regex:'(?i)inv',depth:2}] → passes regex_refusal, written → `hledger bs
  --conf` exits 1 (verified with a scratch conf).
  *Evidence:* confDepth.ts:28-35 catch covers only a throw; hledgerConfModel.ts:83-87;
  hledger_conf.rs:1275-1281 regex_refusal (doc 1253-1255 says it exists so hledger never refuses
  every command); conf_api.rs:365-368 ships raw regex strings. node: '...'.search(new
  RegExp('^assets:[[:alpha:]]+:inv$','i')) → -1; new RegExp('(?i)inv') throws. Scratch crate regex
  1.13.1: `[[:alpha:]]` accepted, `(?i)inv` accepted, `a**` accepted, `a{,2}` rejected, `(a)\1`
  rejected. hledger 1.52: `--depth 'assets:[[:alpha:]]+:inv=2'` clips; `--depth '(?i)inv=2'` → error
  exit 1 via CLI and --conf; `a**` rejected; `a{,2}`/`(a)\1` accepted.
  *Also (E8):* depthLimit constructs new RegExp per override per account (accounts x overrides
  compiles per seed pass); measured 4.7x slower than hoisting but sub-millisecond thanks to V8's
  regexp cache — cleanup, not a defect.

- **A7-3** `web/src/lib/reports/ui/params.ts:511` (CONFIRMED)
  The Accounts tab has a depth control and is documented as `hledger balance -H -M` / `balance -M
  -T` with 'the same --tree --depth', yet CONF_DEPTH_COMMAND seeds only cf/nw, so hledger.conf's
  `[balance] --depth N` moves Net Worth and the terminal but not the Accounts tab — an oversight
  from adding Accounts (d3699d3) after the conf-depth work (e180b28): the code comment, unit test
  and docs still say only Cash Flow and Net Worth have a depth control. The tab's starting depth
  also depends on which tab was visited before, since params.depth is shared and only cf/nw re-seed.
  *Scenario:* `[balance]\n--depth 2` in force; open Reports › Accounts with no depth= in the URL →
  confDepthDefault('accounts') null → depth 3 → three levels, while `hledger balance -M -T --tree`
  under that conf shows two (verified) and the app's own Net Worth tab opens at 2.
  *Evidence:* params.ts:505-511, 158 (accounts depth:true), 264, 520-522; +page.svelte:118-119;
  native.ts:1072; docs/reports/accounts.md:42-44; docs/settings/hledger-config.md:15, 59;
  docs/reports/index.md:34; params.test.ts:376 lists accounts in neither set; git log -S shows
  d3699d3 touched neither map, comment, test nor docs; hledger 1.52 scratch run.

- **E-3** `web/src/lib/settings/hledgerConfStore.svelte.ts:65` (PLAUSIBLE, unreproduced)
  After a successful PUT /api/hledger-conf the store calls journal.refresh() WITHOUT {force:true},
  so if a 5-second poll or window-focus round is in flight it joins that round; when that round's
  GET was served before the PUT's republish, the client keeps the pre-write generation and every
  generation-keyed view (journal rows, AutoRulesPanel counts, Reports --depth key) shows the old
  --auto/--depth state for up to ~5 s while the panel already says saved. Every sibling writer
  (autoRules store, editing, refreshAll) forces.
  *Scenario:* Poller has just sent GET /api/journal; user clicks Save to turn --auto on; PUT
  republishes synchronously and resolves first; `void journal.refresh()` hits journal.svelte.ts:201
  and returns the stale in-flight promise; that round resolves with the pre-write generation → no
  refetch until the next tick. Confirmable with a test mirroring journalRefresh.test.ts's FE-5e
  case.
  *Evidence:* hledgerConfStore.svelte.ts:64-65; journal.svelte.ts:194-196 doc ('force, for any
  caller that has just CHANGED something … FE-5e'), 201; autoRules/store.svelte.ts:44,
  editing.svelte.ts:51/58, refreshAll.ts:127 all force; ran journalRefresh.test.ts in-flight sharing
  tests (4 passed) showing a non-forced refresh joins the round; conf_api.rs:524 republish;
  lib.rs:1515-1517 journal_info unordered vs the PUT.

- **D2-1** `web/src/lib/settings/ui/HledgerConfPanel.svelte:257` (CONFIRMED)
  The client-error list is keyed on the message text and validateForm emits byte-identical sentences
  whenever two overrides in one section share the same trimmed regex and fault (two blank rows;
  three copies of one pattern), so Svelte 5 throws each_key_duplicate (prod too) while rendering:
  the error list never appears and Save silently appears to do nothing (rest of the panel keeps
  working).
  *Scenario:* Settings › hledger config: click 'Add a per-account depth' twice in one section (both
  blank), Save → validateForm returns 5 strings with 3 unique → keyed each throws → settings-
  hledger-conf-client-errors never renders, dirty badge stays, no explanation. No <svelte:boundary>
  or hooks.client.ts handler.
  *Evidence:* HledgerConfPanel.svelte:257 `{#each clientErrors as problem (problem)}`, :78;
  hledgerConfModel.ts:102-105; compiled with svelte 5.57.2 → keyed by the string; each.js:357-363 +
  errors.js:136-147 throw in prod; reproduced by mounting the real panel in the project's jsdom
  harness: 'Uncaught Exception: Svelte error: each_key_duplicate … at indexes 0 and 2'.

- **E-6** `web/src/lib/stores/refreshAll.ts:58` (CONFIRMED)
  REFRESH_TARGETS gained hledgerConf on this branch but not the branch's own /api/auto-rules
  listing, which loads only when `${generation}|${serverUrl}` changes; its request-time fields
  (editable, per-file revision, newRule) survive an explicit Refresh, and after a failed watcher
  reload unbinds the editor (generation unchanged) the panel keeps Edit/Delete/+ Add and every
  Save/Delete answers 501 'editing is not enabled' (not 400), with no re-read since only a 409
  reloads the listing.
  *Scenario:* Settings › Automatic postings open at generation G; an external editor saves
  main.journal with a parse error → watcher reload fails → *guard = None, old snapshot kept,
  generation G unchanged → poll swaps nothing → header Refresh re-reads everything but the listing →
  Edit › Save → PUT → 501 → sentence shown inline, buttons stay, every retry fails until a
  successful reload mints a new generation.
  *Evidence:* refreshAll.ts:58-68 (contract at 32-33: 'A resource missing from it is a resource the
  button silently leaves stale'), reload() 124-163 no case; store.svelte.ts:18-23, 49;
  AutoRulesPanel.svelte:45-49, 63-65, 240; auto_api.rs:208-217, 313-320, 390-392; lib.rs:625-627,
  765, 848-853; main.rs:490; error.rs:110 NOT_IMPLEMENTED; native.ts:1650-1651;
  journal.svelte.ts:80; decodeJournalInfo carries no editable flag.

- **A7-4** `web/src/routes/reports/+page.svelte:110` (CONFIRMED)
  The conf-depth effect returns while `conf === null` without latching and never reads params.depth,
  so the first time hledgerConf.value becomes non-null it runs with confApplied === null and writes
  params.depth = confDepthDefault(...) over any depth the user picked on the Cash Flow / Net Worth
  slider in the meantime — contrary to its own comment and docs/settings/hledger-config.md:69.
  Window is milliseconds on a local engine but minutes with a remote serverUrl or when the first
  conf GET fails (ensure latches loadedKey and only re-fetches on the second generation).
  *Scenario:* `[cashflow] --depth 4`; open Reports on Cash Flow, drag Account depth 3 → 2, then
  /api/hledger-conf resolves → params.depth = 4, report refetches at 4, slider jumps, URL mirror
  writes depth=4, no message.
  *Evidence:* +page.svelte:94, 99, 106-120 (no confApplied write on the null branch; params.depth
  never read); resource.svelte.ts:57, 63-65, 84; hledgerConfStore.svelte.ts:19, 42-47;
  journal.svelte.ts:89; ReportControls.svelte:79-80; DepthSlider.svelte:10; params.ts:519-523;
  docs/settings/hledger-config.md:69.

- **D2-6** `web/src/routes/settings/+page.svelte:39` (CONFIRMED)
  afterNavigate merges searchToParams() into the $state params with Object.assign, and
  searchToParams expresses 'no rule' by omitting the key, so a `rule` focus set by an earlier
  ?tab=auto&rule=N can never be cleared by a later same-route navigation or tab switch; the next
  visit to Automatic postings re-outlines/re-scrolls that rule and the URL mirror writes rule=N
  back.
  *Scenario:* Click a generated posting's rule link → goto('/settings?tab=auto&rule=2'); click the
  notice's 'hledger config' link (same route, component reused) → Object.assign leaves rule:2; click
  the 'Automatic postings' tab (bind, no navigation) → <AutoRulesPanel focus={2}> rings/scrolls rule
  2 and the mirror rewrites ?tab=auto&rule=2. Cleared only by reload or leaving the route.
  *Evidence:* +page.svelte:38-41, 63; params.ts:41, 49-51; SettingsTabs.svelte:25;
  AutoRulesPanel.svelte:257-263, 60; SvelteKit root.svelte reuses the page component on same-route
  navigation; node -e: Object.assign({tab:'auto',rule:2},{tab:'hledgerConf'}) keeps rule:2.
  *Same mechanism reported as:* E-7

### Docs

- **A5-6** `docs/projections/cash.md:171` (CONFIRMED)
  cash.md's three documented hledger commands (bs 171, balance 196, print 208; also index.md:103)
  lack `--auto`, yet cash.md:183-184 promises 'hledger then shows the same figures as the tab' with
  growth as the only difference — false for a scenario with `=` rules (Ledgeline applies them
  always) — and the only test of these commands (`documented()` at projection_files.rs:1461,
  'verbatim in shape') quietly injects `--auto` at 1473 (and 2498 for print), so it can never
  notice. The opt-in gating itself is pre-existing documented design.
  *Scenario:* User with auto.journal-style rules runs cash.md:171's exact command: hledger 1.52
  gives assets:mercury:checking Oct-2026 = -20000 vs the tab's -22000; `is` lacks the payroll-
  tax/sales-commission rows entirely; `print --forecast tag:generated` shows 0 payroll-tax postings
  vs 27. file-format.md:294 mentions adding --auto but cash.md neither says it nor links to it.
  *Evidence:* cash.md:171, 183-184, 196, 208; projection_files.rs:1457, 1471-1473, 2498;
  justfile:136, 155-157; no CI env var; ran the documented commands with and without --auto on a
  scratch copy of the parity fixtures under hledger 1.52 --no-conf.

## Low (28)

### Engine (crates/ledgeline-core)

- **E2** `crates/ledgeline-core/src/auto.rs:78` (CONFIRMED)
  apply_counted_with deep-clones every posting before any rule is tested, calls inherited(posting)
  eagerly for every posting on every rule's scan (self_and_ancestors allocations read only by
  Term::Tag), re-evaluates matches for postings through the first match after the `any` pre-scan,
  drops the clone and deep-clones the transaction again on the no-fire path (contradicting the
  comment 'costs one scan and no copy'), clones postings/tags a third time via `..txn.clone()`, and
  generate() rebuilds the marker String and an Arc<RuleRef> per matched posting rather than per
  rule.
  *Scenario:* 200k txns / 500k postings with --auto and one account-only rule: ~500k×(depth+2) heap
  allocations and ~400k posting-vector copies per snapshot build to produce a journal identical to
  the input except the matched transactions.
  *Evidence:* auto.rs:78, 80-99 (comment 81-82), 107-111, 144-145, 158-166, 445-458;
  wire.rs:284-294; model.rs:29-35, 133-142, 208-240; query.rs:135-142, 165-174 (only Tag reads
  inherited; Compiled exposes no has-tag flag); lib.rs:273-274; forecast.rs:1042;
  edit_api.rs:697-699.

- **A1-7** `crates/ledgeline-core/src/auto.rs:165` (PLAUSIBLE, unreproduced)
  apply_journal's idempotency strip removes generated postings but not the modified-
  transaction/_modified-transaction tags, so re-applying an already-applied journal whose rules no
  longer match would leave the transaction tagged (served in txn tags and matchable by a later `=
  tag:modified-transaction` rule) — but no production caller hands an applied journal back in (every
  Snapshot::from_journal is fed the raw/reparsed journal; apply_in gets the editor's raw txn), and
  the SPA's 'touched by a rule' display keys off posting.generated. Contract gap in a function whose
  comment promises idempotency; the existing test re-applies the SAME rules so it cannot detect it.
  *Scenario:* Only reachable by code that does not exist today (a future caller passing
  snapshot.journal back in).
  *Evidence:* auto.rs:97-105, 153-170 (..txn.clone() at 165); callers traced: lib.rs:275 only non-
  test apply_journal call, fed from editor.journal()/snapshot.raw/vault reparse (lib.rs:146-155,
  303-304, 571-572); edit_api.rs:941-947; parse.rs:3071 generated: None; rowModel.ts:41-53;
  auto_journal.rs:187-189.
  *Same mechanism reported as:* D1-8

- **E1** `crates/ledgeline-core/src/auto.rs:345` (CONFIRMED)
  rule_findings never checks journal.auto_rules.is_empty(): for a journal with zero `=` rules (the
  default for every user) it still walks every posting (firings), runs check_generated_balances
  (display_styles over every amount plus group_residual twice per transaction with a filter that
  rejects everything) and builds the known_accounts HashSet that only the empty rule loop would read
  — four whole-journal passes returning an empty Vec, once per snapshot via the diagnostics prewarm
  and once per GET /api/auto-rules per generation; only apply_journal is guarded (lib.rs:273).
  *Scenario:* Every republish and every Settings › Automatic postings load on a rule-less journal
  pays four extra full passes for a statically empty result.
  *Evidence:* auto.rs:345-402 no early return (347-348 firings, 357 check_generated_balances →
  parse.rs:3221-3232 + assertions.rs:247-276, 383 known_accounts consumed only by the empty loop at
  384); callers wire.rs:503-505, view/checks.rs:98, journal_api.rs:90-120/158-161 prewarm+memo,
  auto_api.rs:231 via compute (no cache); store.svelte.ts:18-24 once per generation.

- **A1-8** `crates/ledgeline-core/src/auto.rs:357` (CONFIRMED)
  rule_findings swallows the Err from check_generated_balances with unwrap_or_default, and because
  unbalanced_where aborts on the FIRST transaction whose generated-group sum overflows i128, one
  such transaction silently drops every Unbalanced finding for every rule in the journal
  (Refused/UnknownAccount survive) while hledger --auto refuses the file; reproduced, but the
  trigger needs generated amounts of magnitude ≥ ~1e18-1e28 beside many-place amounts — contrived
  inputs only, consequence a missing diagnostic.
  *Scenario:* Rules `= revenue:sales / liabilities:tax $10` (plain unbalanced) and `= expenses:food
  / expenses:fee $1e29 / equity:bonus $0.0000000001` with one matching txn each → Dec::add overflow
  in group_residual → check_generated_balances Err → 0 findings; hledger --auto check exits 1 on the
  first rule.
  *Evidence:* auto.rs:357, 341-344; parse.rs:3195, 3232; decimal.rs Dec::add rescale; precedent
  wire.rs:468-475 documents the same swallow as unreachable for real journals. Ran hledger 1.52 and
  a probe against the built rlib: control → 1 finding; overflow → Err, 0 findings, 0 diagnostics.

- **R7** `crates/ledgeline-core/src/auto_form.rs:669` (CONFIRMED)
  unescape() refuses `\` followed by any punctuation outside REGEX_META (`\/`, `\-`, …) even though
  hledger 1.52 and the regex crate both read it as a literal, so a hand-written rule like `=
  ^expenses:a\/b(:|$)` gets form: null and is shown locked ('edit in the file') in the projections
  editor although it is exactly the form the editor itself would write; the cross-table claim
  (hledger_conf's escaped()) is refuted — that table feeds only --alias lines and its `/` is
  documented.
  *Scenario:* Scenario or main journal holds `= ^expenses:a\/b(:|$)` → GET returns form:null,
  unsupported:null (rule fires fine) → ProjectionsTable.svelte:1282-1288 renders the lock badge and
  read-only <pre>; Settings opens it in text mode. False 'cannot be shown as a form'.
  *Evidence:* auto_form.rs:110-112, 663-671 (doc says 'an escaped literal, as the literal'), 646;
  hledger_conf.rs:795 only caller of escaped(); hledger 1.52 scratch: `\/` and `\-` fire as
  literals; regex-syntax 0.8.11 is_escapeable_character accepts every non-alphanumeric ASCII except
  <>; docs/projections/file-format.md:282-285 do not list this case.

- **A3-5** `crates/ledgeline-core/src/hledger_conf.rs:1662` (CONFIRMED)
  hledger 1.52 reads a `--depth` whose value sits on the next line (`--depth\n2` → depth 2,
  verified), but occurrences() reports it as Found::Depth(Bad('')), so Ledgeline shows the depth
  unset, seeds no report from it, emits a misleading badDepth warning ('…every hledger command will
  fail until it is fixed') and a raw-API clear-depth PUT removes nothing yet passes the read-back
  with 200 — a documented, deliberate narrowing (plan 30 l.910-912, code doc l.1632-1634), so what
  remains is the wrong warning text and the UI/terminal disagreement; the UI 'set N' path is benign
  (appends, last wins).
  *Scenario:* Hand-written `--depth\n2`: terminal rolls up to depth 2; GET shows depth null and the
  false warning; PUT {general:{all:null}} returns 200 and writes nothing while depth 2 stays in
  force.
  *Evidence:* hledger_conf.rs:1029, 1181-1185, 1349, 1662; conf_api.rs:467, 589; ran hledger 1.52
  --conf on four scratch layouts and a scratch program against the crate (ConfSettings::read → None
  + BadDepth; without_option unchanged; with_option appends).

- **B-1** `crates/ledgeline-core/src/parse.rs:1985` (PLAUSIBLE, unreproduced)
  The joined open-ended spellings `date:2026-01..` / `date:..2026-04` are refused by period_range
  (hledger 1.52 fires both), but this is a documented, visibly locked limitation rather than a
  silent divergence: the module doc lists open-ended dates as refused, the rule is carried as
  unsupported with the reason, Settings shows a lock badge and projections a warning, and the spaced
  spellings `date:"from 2026-01"` / `date:".. 2026-04"` already work. Cost: a small grammar gap
  (accept an empty side of the `..` split) and an undocumented spelling rule; no Problems-drawer
  diagnostic for the main journal.
  *Scenario:* --auto on with `= date:2026-01..`: hledger adds the postings to every transaction
  on/after 2026-01-01, Ledgeline adds nothing and shows the lock.
  *Evidence:* parse.rs:1985-1986, 1949, 1870-1875; query.rs:31-36, 318-319; docs/journal/automatic-
  postings.md:64; auto_api.rs:279; AutoRulesPanel.svelte:269-270, 305; hledger 1.52 scratch run
  fires both; `date:"from 2026-01"` accepted by both.

- **L2** `crates/ledgeline-core/src/projections/forecast.rs:1732` (CONFIRMED)
  Forecast-scoping is decided by three independent text approximations (substring at
  forecast.rs:1732, exact-term equality at auto_form.rs:466-467, includes() at
  ProjectionsTable.svelte:1290) rather than the compiled query: `tag:_generated` / `tag:generated`
  are forecast-only in hledger yet get the warning, the badge and the button (Save then writes a
  redundant second tag term), while `not:tag:_generated-transaction` (recorded-only in hledger,
  never fires on forecasts here) passes all three sniffs and a dead rule goes unflagged.
  *Scenario:* `= a tag:_generated` → spurious warning/badge/button; clicking it writes `^a(:|$)
  tag:_generated-transaction tag:_generated`. `= a not:tag:_generated-transaction` → no warning
  though it never fires in Ledgeline and fires on recorded transactions only in hledger.
  *Evidence:* forecast.rs:1732; auto_form.rs:466-467, 461, 626, 630; ProjectionsTable.svelte:1290;
  query.rs:288, 300-302, 353-358; projections_api.rs:1250-1253; hledger 1.52 scratch runs for all
  four spellings.
  *Same mechanism reported as:* D1-6

- **B-3** `crates/ledgeline-core/src/projections/serialize.rs:1734` (CONFIRMED)
  confirm_auto_rules re-parses the just-written text a second time WITHOUT doc.inherited three lines
  after confirm_round_trip parsed it with the styles; harmless today (only rule.raw is compared and
  the unstyled read is the more lenient one) but an inconsistent application of the branch's with-
  styles rule and a wasted full second parse per save.
  *Scenario:* No wrong output; every save/create parses the scenario twice and discards all but the
  raw strings that confirm_round_trip's written.auto_rules already holds.
  *Evidence:* serialize.rs:168, 1706-1707, 1734-1738, 1770-1771; parse.rs:754-762, 2857-2867; no
  test covers a styled journal with `=` rules through write_scenario.

- **L8** `crates/ledgeline-core/src/query.rs:55` (CONFIRMED)
  The reason a query is unsupported is a prose String (seven lower-case clauses) with no code; it
  crosses the wire as unsupported: Option<String> and is spliced into FOUR differently framed
  sentences (forecast.rs:1729, auto_form.rs:442, AutoRulesPanel.svelte:305,
  ProjectionsTable.svelte:1321) plus a tooltip; no caller can branch on the kind, tests assert only
  presence or the frame, and the repo's ConfWarning code()+message() pattern is the typed model it
  departs from.
  *Scenario:* Cleanup: a reason reworded as a full sentence breaks four frames at once with no test
  catching it; clients cannot distinguish a user-fixable unclosed quote from an engine limitation.
  *Evidence:* query.rs:55, 113-115, 276/309/315/319/323/341/358; forecast.rs:1727-1729;
  auto_form.rs:441-442; AutoRulesPanel.svelte:270, 305; ProjectionsTable.svelte:1321;
  auto_api.rs:279; projections_api.rs:372; hledger_conf.rs:1095-1108; conf_api.rs:199-201, 389-390.

- **R6** `crates/ledgeline-core/src/query.rs:250` (CONFIRMED)
  query::tokens (engine; drops quotes, Err on unclosed) and auto_form::raw_terms (editor read-
  back/write; keeps quotes, None on unclosed) are the same quote-aware splitter written twice, tied
  together only by a doc comment; they agree today (verified over 11 inputs) but any quoting change
  to one must be mirrored by hand or the editor shows a rule locked that the engine fires (or vice
  versa). query.rs:214 payee duplicates subscriptions.rs:214 payee_of (functionally identical since
  descriptions are stored trimmed).
  *Scenario:* Cleanup: convention-only agreement between compile→tokens and
  split_query/normalized_extra→raw_terms.
  *Evidence:* query.rs:247-282; auto_form.rs:470-504; scratch rustc run of verbatim copies;
  parse.rs:1596 trims descriptions; subscriptions.rs:214-220 pre-existing on main.

- **A4-7** `crates/ledgeline-core/src/query.rs:327` (CONFIRMED)
  `inacct:` and `inacctonly:` (hledger-web register-focus OPTIONS, which hledger 1.52 parses to Any
  plus a QueryOpt the matcher never sees) fall into the default `_ => Term::Acct(pattern(rest)?)`
  arm and become an account regex, so a rule whose only account-ish term is the option silently
  never fires (no lock, no warning) and `not:inacct:…` fires where hledger never does; the `empty:`
  half is refuted (hledger 1.52 reads it as acct:empty: too). Nobody writes these in an `=` rule on
  purpose.
  *Scenario:* `= inacct:expenses:food desc:rent / (budget:fired) 1`: hledger generates 2 postings,
  Ledgeline 0 with unsupported null; `= not:inacct:zzz desc:rent`: Ledgeline 2, hledger 0.
  *Evidence:* query.rs:292, 322, 325-327, 239, 134; auto.rs:84; hledger-lib 1.52 Query.hs:294-298,
  327, 279; ran hledger 1.52 and a throwaway crate on scratch journals — counts as quoted.

- **E5** `crates/ledgeline-core/src/reports/account_report.rs:211` (CONFIRMED)
  account_report_from walks every column map once per section (6 passes, including sections that
  turn out empty), classifying each entry on every visit through AccountSection::of's up-to-5
  sequential memo probes, and clones each matched (String, MixedAmount) entry though `direct` is
  owned and could be partitioned by move in one pass with a single resolve per entry; `known` is re-
  filtered 6 times and the opening map walked and cloned twice more.
  *Scenario:* 6*E entry visits, up to 30 hash lookups per expense (account, column) entry, E
  String+BTreeMap clones per GET /api/reports/accounts and per projections run with accounts;
  milliseconds, not a stall.
  *Evidence:* account_report.rs:56, 67-78, 189, 198-205, 209-218, 238, 251; account_types.rs:77
  root_type, 365-375, 427-440; reports_api.rs:3343; projections_api.rs:1353 → real_side.rs:682-684;
  mixed_amount.rs:20.

### Server (crates/ledgeline-server)

- **E3** `crates/ledgeline-server/src/auto_api.rs:314` (CONFIRMED)
  Files::new reads, decodes and fingerprints EVERY writable journal file on each GET /api/auto-rules
  (and each PUT's answering listing), yet only main's revision and the revisions of files holding an
  `=` rule are consumed; each file is also hashed twice because read_journal already returns a
  Revision with the identical fingerprint which Files::new discards and recomputes.
  *Scenario:* Five-file layout with rules only in main: every listing (per generation while the
  panel is open, plus twice per save) reads all five files in full (multi-MB year files included)
  and emits only main's token.
  *Evidence:* auto_api.rs:308-323 (filter only target.writable; `let (text, _) = read_journal(...)`
  then Fingerprint::of_bytes at 319); consumers 262, 272; budget_api.rs:1516-1531 read_journal →
  read_pinned; pinned.rs:162; vault/mod.rs:177 token(); journals.rs:104-114, 137, 159-173;
  store.svelte.ts:18-22, 44-45.

- **R2** `crates/ledgeline-server/src/auto_api.rs:475` (CONFIRMED)
  save_rule reproduces save_budget's write orchestration verbatim in three segments (writable-target
  refusal 405-416, VaultError::Changed/Internal mapping 475-483, reopen_editor tail 485-487) and
  Files::new (302-325) is another instance of the per-target read-and-fingerprint listing; the leaf
  helpers are shared via budget_api but the orchestration is now a 4th-5th copy
  (alias/account/prices/budget/auto/conf) with no extracted helper.
  *Scenario:* Cleanup: a change to the write discipline (temp-path leak rule, refusal wording, post-
  write reopen) must be made in six modules; the copies already drift in the refusal noun.
  *Evidence:* auto_api.rs:410-415 vs budget_api.rs:881-886; 475-483 byte-identical to
  budget_api.rs:951-959 (also prices_api 799-806, conf_api 498-505); 485-487 vs 965-967;
  auto_api.rs:90 imports the shared leaves; Files::new 302-325 vs budget_api 611-632 (and
  account_api:356, alias_api:309, projections_api:1940, import_api:1515).

- **S6** `crates/ledgeline-server/src/conf_api.rs:515` (CONFIRMED)
  ConfState's derived fields (revision, settings, hijacked_by) are computed by hand in
  ConfState::read (104-111) and run_put's answer (511, 515-522) with no from-text constructor; today
  they agree, so the PUT-body-vs-GET drift and the panel re-seed are a latent cost.
  *Scenario:* Cleanup: a future derived field or fingerprint change missed in one constructor makes
  PUT's answer differ from the next GET and HledgerConfPanel's keyOf latch re-seeds the form,
  discarding edits.
  *Evidence:* conf_api.rs:104-111, 511, 515-522; no from_text helper; HledgerConfPanel.svelte:44-55,
  86.

- **R3** `crates/ledgeline-server/src/conf_api.rs:528` (CONFIRMED)
  conf_api declares a private fn stale(handle) (a 7th `fn stale` in the server) whose 409 text
  differs from budget_api::stale, plus another copy of the write-error mapping; the 'one-message
  rule' clause is refuted (that doc is per-endpoint and conf_api honours it; its wording is copied
  from import_api's alias write), and the UI never shows the server text.
  *Scenario:* Cleanup only; the wording divergence is observable only on the raw API. A shared
  helper would need a non-quoting variant since the handle is server-derived.
  *Evidence:* conf_api.rs:528-533, 498-505, 445, 489-495; budget_api.rs:1556-1565; git grep 'fn
  stale' main → six private copies with four wordings; import_api.rs:5737/5795 identical text;
  hledgerConfStore.svelte.ts:68-70; HledgerConfPanel.svelte:147-153.

- **C-4** `crates/ledgeline-server/src/edit_api.rs:699` (CONFIRMED)
  With --auto in force, a posting-account PATCH maps served indices through auto::apply_in, which
  propagates DecError::Overflow as a 500, while the served row came from apply_journal, which
  swallows the same error and serves only the written postings; recategorizing a posting on a
  transaction whose rule arithmetic overflows i128 fails with 500 'decimal arithmetic overflow'
  instead of the identity mapping the served row implies. Reachable only with absurd magnitudes
  (~1.7e38); nothing is written.
  *Scenario:* `= expenses / (virtual:tax) *2` over `expenses:big
  $99999999999999999999999999999999999999`: row served with 2 postings and editable chips; click a
  chip → PATCH → written_positions → apply_in Err(Overflow) → 500. Works with --auto off.
  *Evidence:* edit_api.rs:698-699; auto.rs:123-124, 171-172, 200, 91, 463, 524; decimal.rs:129-132,
  324; error.rs:112; lib.rs:274-275; AccountsCell.svelte:7-10; editMapping.ts:305-307; scratch crate
  output as quoted; no RuleProblem variant covers overflow.

- **D1-7** `crates/ledgeline-server/src/lib.rs:267` (CONFIRMED)
  Every Snapshot::from_journal (every in-process server test state and every republish) runs
  auto_setting → locate_conf_for → locate_conf, which walks the REAL directories above the vault
  root with fs::metadata for up to 63 levels; default_app_dirs() and
  LEDGELINE_IGNORE_USER_HLEDGER_CONF blank only the ~/.hledger.conf/XDG tiers, so an hledger.conf in
  any real ancestor of a fixture's directory (the repo's ancestors, or $TMPDIR's ancestors for the
  temp-dir auto/conf tests) flips the inForce/conf/handle answers and, where the fixture carries `=`
  rules, the served postings. The above-root walk pre-dates the branch; new is running it inside
  every snapshot build and letting it rewrite the served journal. Test-hermeticity gap, not a
  product bug (hledger itself walks upward).
  *Scenario:* `printf -- '--auto\n' > $TMPDIR/hledger.conf` then `cargo test -p ledgeline --test
  auto_endpoints without_auto_the_rules_are_listed_but_never_applied` fails (inForce true, 4
  postings instead of 2); an EMPTY conf there already flips conf_endpoints' null-handle assertions;
  a conf at the repo's ancestor makes settings.e2e.ts's 'settings-auto-off' fail despite the ignore
  env var. No such file exists on this machine today; CI runs in nix.
  *Evidence:* lib.rs:267-274, 316-318, 586-590; location.rs:52-56; auto_api.rs:124;
  hledger_conf.rs:123, 445-455, 600-624; local.rs:78-88, 237-238; app_dirs.rs:89-90;
  tests/common/mod.rs:44-60; auto_endpoints.rs:25-27; conf_endpoints.rs:52; live run of the branch-
  tip server with the ignore env var set and an ancestor hledger.conf → inForce true, 4 postings
  served.

- **L7** `crates/ledgeline-server/src/projections_api.rs:351` (CONFIRMED)
  One rule has two hand-mirrored outbound shapes (WireAutoRule vs WireRule), two inbound twins
  (AutoRuleIn vs RuleIn) each with its own copy of the 'exactly one of raw and form' match and
  identical error string, two TS interfaces named AutoRule and two decoders named decodeAutoRules;
  the block field already drifted in type (Option<usize> vs usize). The badge-markup claim is
  overstated (parallel, not duplicated).
  *Scenario:* Cleanup: a rule field must be added in two Rust wire structs, two inbound structs, two
  match arms, two TS interfaces and two decoders.
  *Evidence:* projections_api.rs:351-365, 873-889, 1247-1258; auto_api.rs:166-186, 375-380, 530-543;
  projections/types.ts:236; autoRules/types.ts:18; nativeDecode.ts:2523; autoRules/decode.ts:78.
  *Also (S5):* WireRule's journalId/revision are two nullables projected off one Option;
  WireFileRevision already exists; file is re-derived by file_id though equal to journalId when set.
  *Also (R5):* file_id re-implements journals::identify/relative_id (third copy; dirscan.rs:338 pre-
  existing); Files::new already receives the id from location.targets() before filtering.
  *Also (S7):* decode.ts re-declares primitives nativeDecode.ts holds privately and
  journalViewDecode.ts (also new) holds byte-identically and partly exports; three copies have
  already diverged in strictness (the V18 nullableStr gap).
  *Also (S8):* precision is always quantity.places outbound and dead inbound; projections inbound
  query/unsupported and accounts.mode are compat shims for senders that never shipped; impl
  From<RuleEditError> for AppError is never invoked.

- **B-2** `crates/ledgeline-server/src/projections_api.rs:931` (CONFIRMED)
  The 400 guard on accounts.mode was removed (any value now ignored while deny_unknown_fields still
  400s an unknown KEY); a deliberate, commented and tested compatibility shim for a pre-2026-10-28
  page, not a defect.
  *Scenario:* POST /run with accounts.mode 'cumulative' → 200 per-section reading; no wrong money.
  *Evidence:* projections_api.rs:924-934; main's balance_mode deserializer; test
  an_old_page_s_accounts_mode_is_ignored (ran, passed); native.ts:790 new client sends only empty.

- **S4** `crates/ledgeline-server/src/reports_api.rs:3306` (CONFIRMED)
  GET /api/reports/accounts duplicates the cash-flow handler's 18-line CashFlowWindow → (from, to,
  interval, depth) block byte for byte (3306-3323 == 3382-3399, verified with diff) and rebuilds a
  CashFlowQuery field by field just to call resolve; a `fn bounds(self)` on CashFlowWindow would
  delete both copies.
  *Scenario:* Cleanup: the trailing-window conversion (incl. the silent empty `from` when the bucket
  list is empty) must be fixed twice.
  *Evidence:* diff of the two ranges is empty; main has one copy (3226); impl CashFlowWindow holds
  only resolve; 3297-3304 rebuilds the query.

- **R8** `crates/ledgeline-server/src/reports_api.rs:3330` (CONFIRMED)
  The accounts handler inlines a declared ∪ posted-to BTreeSet although pub
  wire::journal_to_accountnames (already used at lib.rs:281 and account_class.rs:52) is a drop-in
  yielding identical rows (shown() expands ancestors anyway); the branch now has FIVE spellings of
  'every account the journal knows' (reports_api 3330, real_side.rs:101-110 / known_accounts(),
  forecast.rs:429, auto.rs:407, wire.rs:1398) differing in ancestors, periodic postings, forecast
  postings and generated-posting exclusion — which is the mechanism behind the L6 inconsistency.
  *Scenario:* Cleanup: five functions to read to know whether the Accounts show-empty list, the
  Projections show-empty list, the projections unknown-account warning and the Settings finding
  agree (they do not).
  *Evidence:* reports_api.rs:3330-3342; wire.rs:1398-1416; real_side.rs:101-110, 120;
  forecast.rs:429-449; auto.rs:407-426; account_periods.rs:177, 190-194; account_report.rs:212-218,
  242; lib.rs:274-275.

### Web (web/src)

- **E6** `web/src/lib/autoRules/store.svelte.ts:45` (CONFIRMED)
  One rule save makes the server build the auto-rules listing three times: the PUT's own answer
  (decoded and discarded), a GET from reloadAutoRules keyed on the pre-save generation, and a GET
  from the panel's $effect when the forced journal round moves the generation; each listing re-reads
  and fingerprints every writable file and walks every transaction for generated_counts and
  rule_findings.
  *Scenario:* Any successful Save/Add/Delete in Settings › Automatic postings; no wrong output
  (dataView keeps the held payload), wasted server/network work; GET #2's body may also be dropped
  by the stale-token guard.
  *Evidence:* store.svelte.ts:43-45, 19-21, 27-28; journal.svelte.ts:90 generation written after the
  await; auto_api.rs:485, 488, 208-231; resource.svelte.ts:83; createResource has no setter (31-47).
  *Same mechanism reported as:* S3

- **A6-3** `web/src/lib/export/xlsx.ts:901` (CONFIRMED)
  The Δ marker is prefixed as a quoted literal onto a ONE-section number format, so a negative Δ
  renders in Excel/LibreOffice as `-Δ $3,200.00` (minus before the marker) while the screen renders
  `Δ $-3,200.00`; value correct, glyph order differs.
  *Scenario:* Accounts .xlsx with any assets/liabilities row whose balance fell over the window
  (routine) → cell -50000 with numFmt `"Δ ""$"#,##0.00` → Excel shows `-Δ $50,000.00` vs screen `Δ
  $-50,000.00`. No test covers a negative Δ (export.test.ts:104 only finds 3200).
  *Evidence:* xlsx.ts:901, 100, 81-82 (no ';' section), 125; money.ts:225, 236;
  AccountReportView.svelte:48; account_report.rs:393, 402; two-section idiom exists at xlsx.ts:830.
  Reasoned from documented Excel semantics, not executed.

- **L5** `web/src/lib/projections/ui/ProjectionReports.svelte:166` (CONFIRMED)
  Runway divider/marker derived from cash.history.length while the statement views use
  projection.projectedFrom (two sources of truth equal only by engine construction, asserted nowhere
  web-side); Projection::net_worth (opening/values/history, incl. a net_worth_priced run per real-
  side build) is computed but never serialized — pre-existing on main, not new to this branch.
  *Scenario:* Cleanup: if an engine ever sent cash.history shorter than lookback,
  PeriodLineChart.svelte:107 would plot forecast figures under actual-month labels with no decoder
  error (exactly what the test at ProjectionReports.svelte.test.ts:248-262 feeds it).
  *Evidence:* ProjectionReports.svelte:78, 166, 170-171 vs 183/187/191/204; projections.rs:814-822,
  996-1000; projections_api.rs:586-615 no net_worth; nativeDecode.ts:2678 checks only projectedFrom
  > columns.length; main already had net_worth_history.

- **A6-7** `web/src/lib/projections/ui/ProjectionReports.svelte.test.ts:83` (CONFIRMED)
  The 'draws no divider when the Actuals control asks for none' test overrides
  lookback/projectedFrom/cash but not balanceSheet, so the chart is handed 4 labels against a
  2-value series; PeriodLineChart silently pads with 0 and the test asserts only the projected-rule
  count, so neither runway-chart test would notice a series shorter than its labels (e.g. line 166
  regressing to drop history).
  *Scenario:* Test quality: fixture internally inconsistent; chart under test draws 14800/19600
  under Jun/Jul and 0 under Aug/Sep and passes. Fix: 2-column sheet via widenSheet(...,
  PROJECTION_COLUMNS.slice(2)) and/or assert point count equals labels.length.
  *Evidence:* Test line 83-85; projectionFixture.ts:23-28, 69; ProjectionReports.svelte:78, 166,
  170; PeriodLineChart.svelte:107 pads (header says a missing value is never drawn as zero); no test
  asserts the runway chart's point count. Ran the test file: 5 passed.

- **E7** `web/src/lib/settings/hledgerConfStore.svelte.ts:64` (CONFIRMED)
  After PUT /api/hledger-conf answers with the full WireConf, save() issues a redundant GET that re-
  locates/re-reads/re-parses the config; until it lands the resource holds the pre-save conf while
  the form was re-seeded from the PUT body, so the panel briefly reads dirty ('unsaved' badge,
  holdUnsavedEdits asserted, Save re-enabled carrying the OLD revision → a click in that window gets
  a 409 for the user's own write).
  *Scenario:* Toggle --auto and Save; timing-dependent window before the local GET resolves.
  *Evidence:* hledgerConfStore.svelte.ts:62, 64, 52; resource.svelte.ts:82-84; conf_api.rs:524, 526,
  409-417; lib.rs:704-716; HledgerConfPanel.svelte:33, 57, 61, 83-84, 247, 251-252;
  hledgerConfModel.ts:128-129, 139-142; conf_api.rs:446-447.

## Fixed on the branch before the PR (13 high)

For the record, the high-severity findings and the commits that fixed them:

- **A1-1** `crates/ledgeline-core/src/auto.rs:494` — the amountless leg of a `*N` rule was inferred from raw quantities, not at cost as hledger does. _Fixed in: TBD_
- **C-1** `crates/ledgeline-core/src/auto_form.rs:356` — rules were rendered with a hard-coded `.` decimal mark, so comma-mark journals read them 10x/100x. _Fixed in: TBD_
- **A3-1** `crates/ledgeline-core/src/hledger_conf.rs:1174` — an explicit `[general]` section was not treated as the general section. _Fixed in: TBD_
- **SW-3** `crates/ledgeline-core/src/parse.rs:1762` — balance assertions on `=` rule postings were dropped, and deleted on re-save. _Fixed in: TBD_
- **A4-2** `crates/ledgeline-core/src/parse.rs:1772` — a `*FACTOR` followed by an exponent, cost or assertion was re-denominated in the empty commodity. _Fixed in: TBD_
- **D1-1** `crates/ledgeline-core/src/projections/serialize.rs:718` — removing the last two adjacent `=` blocks produced overlapping splices and panicked. _Fixed in: TBD_
- **A4-1** `crates/ledgeline-core/src/query.rs:201` — `amt:` matched every multi-commodity or empty leg, unlike hledger. _Fixed in: TBD_
- **A4-4** `crates/ledgeline-core/src/query.rs:311` — `real:yes` / `real:true` selected the opposite postings from hledger, which accepts only `1`. _Fixed in: TBD_
- **A5-7** `crates/ledgeline-server/src/projections_api.rs:399` — rule percentages were signed with the main journal's account types, ignoring the scenario's own declarations. _Fixed in: TBD_
- **A5-1** `crates/ledgeline-server/src/projections_api.rs:1248` — the projection run parsed raw rules without commodity styles, so `$1,500` became `$1.500`. _Fixed in: TBD_
- **A2-8** `web/src/lib/autoRules/ui/AutoRulesPanel.svelte:75` — a one-posting percent rule could be saved, making hledger refuse the whole journal under `--auto`. _Fixed in: TBD_
- **D2-4** `web/src/lib/autoRules/ui/AutoRulesPanel.svelte:120` — the rule editor was keyed by listing index, so Save could overwrite a different rule after a refresh. _Fixed in: TBD_
- **A2-3** `web/src/lib/autoRules/ui/AutoRulesPanel.svelte:132` — a failed Delete reopened the editor on the deleted rule's text, and Save replaced whichever rule then held that index. _Fixed in: TBD_
