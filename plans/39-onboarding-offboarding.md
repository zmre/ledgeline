# Plan 39: Onboarding and offboarding (QIF, posting-row CSV, File → New)

Status: Phases 1, 2, 4, 5 and 6 implemented 2026-11-12 (see "Implementation notes" at the end). Phase 3, the Banktivity round trip, is Patrick's manual step and is still open. Research is done (see the Research appendix); don't redo it.

## Context

Patrick wants to try Banktivity: move his hledger books into it, then move them back. Two reasons: to see how Ledgeline compares, and to give someone a path to try Ledgeline. That needs:

- an export Banktivity can import;
- an import of Banktivity's export that Ledgeline can use as a starting point.

The same machinery should grow into the long-standing `TODO.md` "File → New" item, which says:

> "- feat: File -> New
>   - Here I'm assuming we're setting up a new set of journal files, chart of accounts, etc. Probably we prompt with some questions and use an empty folder as a starting point and then create a skeleton so someone can start using us to track things. We should have a default chart of accounts for individuals and another for businesses and then we should allow them to start with an empty set of accounts to add their own."

The new wizard adds a second path, "start from another app's export". The longer-term goal is Quicken, Monarch, Xero and similar apps.

**Rules for the implementing agent:**
- Never read Patrick's real journals. Build and test with `fixtures/` only. Patrick runs the real export himself.
- Prefer native Rust over hledger subprocesses. Patrick accepts hledger calls where they're unavoidable, but nothing in this plan needs one; tests may still use hledger as an independent check.

---

## 1. Ledgeline vs Banktivity (≈1 page)

| Area               | Banktivity 10 (May 2026)                                                                 | Ledgeline                                                                                         |
|--------------------|-------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------|
| Platforms          | Mac, iPhone, iPad; E2E-encrypted CloudKit sync                                            | Mac desktop (arm64/x86_64); files sync however you like (iCloud, git)                             |
| Price              | Subscription $60 / $80 / $100 per year (Bronze / Silver / Gold), or $6.99–$10.99 monthly  | Free; published on crates.io                                                                      |
| Data format        | Proprietary `.bank8` package (Core Data SQLite at `StoreContent/core.sql`)                | Plain-text hledger journals                                                                       |
| Bank feeds         | Plaid by default (since 9.5); Yodlee; direct OFX (Mac)                                    | File import only (CSV/TSV/OFX/QFX/QBO/XLS/XLSX/ODS through rules); SimpleFIN is an open question (`TODO.md`) |
| Accounting model   | Single-entry registers plus categories; splits; transfers                                 | Full double-entry: any number of postings, any accounts                                           |
| Budgeting          | Envelope budgeting with refill targets                                                    | hledger periodic budgets and a budget report; no envelopes                                        |
| Scheduled txns     | Yes, with rich recurrence rules                                                           | Periodic transactions, projections and scenarios (`hledger --forecast` compatible)                |
| Investments        | Securities, cost basis, performance, price downloads (Silver tier and up)                 | Commodities, prices, holdings report; no automatic price download; OFX investment statements refused |
| Multi-currency     | Gold tier only                                                                            | Native (hledger commodities)                                                                      |
| Reports            | Income/expense, net worth, payee, category, cash flow, investment                         | Balance sheet, income statement, cash flow, holdings, budget, runway, comparison, SaaS/KPI metrics, Insights, Subscriptions; xlsx export |
| Business           | Check printing; "family or small business" users; no AR/AP or invoicing                   | Classes, customers and vendors (QuickBooks parity), SaaS metrics, QBO journal import and export   |
| Reconciliation     | Yes, auto-reconcile                                                                       | Cleared/pending status, balance assertions, transfer matching                                     |
| Attachments, tags  | Yes / yes                                                                                 | Tags yes; attachments: not checked                                                                |
| Mobile             | Yes                                                                                       | No                                                                                                |
| Lock-in            | High: export is QIF/TXF/CSV only, and QIF loses data (see §2)                             | None: the text file is the database                                                               |

**Pitch for a Banktivity user:** real double entry, multi-currency without paying for the top tier, stronger business and forecasting reports, no subscription, and data that can't be held hostage.

**Banktivity wins:** mobile, automatic bank sync, envelope budgets, investment performance and price feeds. These are the gaps a switcher will notice.

Keep this comparison in `plans/`, not on the published docs site, until Patrick decides whether to publish a "Switching from X" page.

---

## 2. Format findings and what each loses

**Banktivity imports** QMTF, QFX, OFX, QIF and CSV. **It exports only QIF, TXF (tax) and CSV.** CSV export is one account at a time with no splits. So **OFX is not even an option coming back out of Banktivity, and QIF is the only rich path in both directions.**

### If we exported OFX for Banktivity, it would lose

- **Categories.** Everything lands uncategorised. OFX has only a payee, memo and TRNTYPE.
- **Splits.** Each transaction has one amount.
- **Transfer links.** Each account's file stands alone, so both sides import as unrelated transactions.
- **Account hierarchy and types.** OFX does carry a bank vs. credit card distinction.
- **Cleared/reconciled flags and tags.**
- **Equity and opening balances.** Only a ledger balance is carried.
- **Expense and income accounts.** They have no OFX form at all.

OFX only works for "here are my bank transactions, re-categorise them". **Don't use it for migration.**

### If we export QIF (recommended), it keeps

- account list and types (Bank, CCard, Cash, Oth A, Oth L);
- category list, including `Parent:Child` (the same `:` hierarchy hledger uses);
- splits (`S`/`E`/`$`);
- transfers (`L[Account]`);
- cleared/reconciled status (`C*`/`CX`);
- payee and memo;
- one tag dimension, as the QIF class `/Class`.

### QIF still loses or degrades

- **Multiple commodities.** QIF has no currency field, so it needs one file per currency (Banktivity does the same).
- **Investments.** Lots, cost basis and prices don't survive well. Use `!Type:Invst` only where an action maps cleanly (Buy, Sell, Div); otherwise fall back to cash movement.
- **Balance assertions, prices, periodic/forecast transactions, budgets, account tags** (`kpi:`, `cfgroup:` …) and **multiple tags.**
- **Postings with no bank-like account.** For example an expense-to-expense reclassification, or an equity entry. Every QIF transaction must live in an account register, so these need a "Ledgeline Adjustments" holding account or they are dropped. Opening balances become QIF `Opening Balance` records.
- **Posting comments.** They partly survive as split memos.
- **Auto-postings.** Apply them before export (see Phase 1).
- **Banktivity's QIF re-export also has known problems:**
  - transfers appear in both accounts, so we must dedupe them;
  - splits that include transfers are reported to break;
  - each per-currency file repeats the full account and category list.

### If we import Banktivity's QIF, we lose

- budgets, schedules, attachments and reconciliation history (none of these are exported);
- investment fidelity;
- currency, which must be inferred per file.

Categories, splits and transfers come through.

### The other apps, generalised

| Source                        | Best export                                     | Rebuild double entry?                                                     |
|-------------------------------|-------------------------------------------------|---------------------------------------------------------------------------|
| Quicken Windows               | QIF                                             | Yes, after deduping transfers                                             |
| Quicken Mac                   | QMTF (a QIF variant) or CSV; no QIF since 2007  | Mostly, via QMTF treated as QIF                                           |
| Moneydance, GnuCash           | QIF; GnuCash multi-line CSV                     | Yes                                                                       |
| Xero                          | Journal Report / GL Detail CSV                  | Yes, by grouping rows on `Journal ID`                                     |
| QuickBooks Online             | Journal report XLSX                             | **Already supported** (`crates/ledgeline-core/src/qb_journal.rs`)         |
| YNAB, Lunch Money, Actual     | CSV with split markers or parent IDs            | Partly; heuristics needed                                                 |
| Monarch, Copilot              | One row per transaction (account + category)    | Single entry: map categories to accounts and pair transfers. Monarch's balance-history CSV gives balance assertions. |

**About CSV and transactions with more than two accounts:** a CSV can hold them if it uses **one row per posting with a shared transaction key**. Xero (Journal ID), hledger `print -O csv` (`txnidx`) and GnuCash (ID on the first row) meet that. Monarch, Copilot and our current CSV rules path do not: they are one row per transaction, which caps it at two legs. **hledger cannot import a posting-per-row CSV, even its own `print -O csv` output.** Its CSV rules make one transaction per record, and `merge N` (1.99.5+) only joins a fixed N rows. So Ledgeline needs its own grouped-CSV importer (Phase 4).

**MYE** is MYOB's accountant-extract format: a zip containing `MYOBAO.txt`, used when moving a general ledger into MYOB practice software. Xero offers it only for that purpose. **Ignore it.**

---

## 3. Recommendation

There are three importer shapes. Each source app becomes a *preset* on one of them, not a new parser.

1. **QIF, both ways (do first).** This covers Banktivity, Quicken (Windows, and Mac via QMTF), Moneydance and GnuCash.
2. **Posting-row CSV, both ways.** Rows are grouped by a key column, a key carried down to blank rows, or a balance heuristic. Presets: Xero, GnuCash, hledger `print -O csv`, YNAB.
3. **Single-entry CSV.** The existing rules pipeline, plus a category→account mapping, transfer matching (`matching.rs`) and balance assertions. Presets: Monarch, Copilot, Lunch Money.

The same importers feed a **File → New wizard** with two paths:

- **Start fresh.** Ask:
  - individual or business;
  - base currency (writes a `D` directive);
  - folder.

  Then write `main.journal` and `accounts.journal`, using a template chart of accounts (individual or business) or an empty one, with `type:` declarations.
- **Start from another app.** Pick a source app and its export file. Then:
  - QIF `!Account` types and categories, or a Xero chart-of-accounts CSV, *derive* the account tree;
  - the user reviews and renames it in the existing alias-mapping UI from the QuickBooks import;
  - the wizard writes the account declarations, the opening balances (to `Equity:Opening Balances`) and the transactions.

OFX/QFX can seed a skeleton too: one account per statement, plus an opening balance worked out as LEDGERBAL minus the statement total, followed by the normal rules import. It is the weakest start, but it is the right one for people coming straight from a bank.

The wizard must not create `ledgeline.toml`; that file is hand-made and never auto-created (`docs/settings/settings-files.md`). Preferences go to `ui-prefs.toml` only.

---

## 4. Implementation phases

**Phase 1: QIF export.** Hand-written; the Rust QIF crates are immature.
- New `crates/ledgeline-core/src/export/qif.rs`. It is pure Rust with **no hledger subprocess**. It reads the journal with the native parser (`parse.rs` `parse_journal_in`), applies auto-postings with `auto.rs` (which mirrors `hledger print --auto`), and takes account types from `reports/account_types.rs`.
- Classify each account:
  - Asset (Cash) → `Bank`/`Cash`;
  - Liability → `CCard`/`Oth L`;
  - other Assets → `Oth A`;
  - Revenue/Expense → categories.
- Write one record per register-account side, with `S` lines for the other postings and `[Account]` for transfer legs. Write each transfer **once** and let the target app mirror it. Verify that choice in Phase 3, since Banktivity auto-matches transfers.
- Write `!Type:Cat` and `!Account` lists, MM/DD/YYYY dates with 4-digit years, `C` status, one file per commodity, and a holding account for transactions with no bank-like posting.
- Produce a loss report (the counts of everything dropped). Showing users what will be lost is part of the pitch.
- CLI: `ledgeline export --qif out/ -j main.journal`, modelled on the import subcommand in `crates/ledgeline-server/src/main.rs`. Then add a native menu item, File → Export → QIF…, in `crates/ledgeline-server/src/gui.rs` `build_menu` (muda/tao; the File menu has Open journal…, Open Recent, Close and Quit today).

**Phase 2: QIF import (whole-book migration).**
- New `crates/ledgeline-core/src/qif.rs` parser.
- A `qif_import.rs` that mirrors `qb_import.rs`:
  - map categories and accounts to journal accounts with `alias` lines;
  - generate stable `id:` tags (QIF has no IDs, so hash date, account, amount, payee and occurrence index) so re-import dedupes;
  - collapse mirrored transfers;
  - turn opening-balance records into equity postings.
- Reuse the API and UI shape of `crates/ledgeline-server/src/qb_journal_api.rs` and its mapping screen. Docs follow `docs/imports/quickbooks.md`.
- Handle date ambiguity (2- or 4-digit years, no leading zeros, `'` year separator) by detecting the format across the whole file, and ask the user if it's ambiguous.
- Accept `.qif` and `.qmtf`.
- Fixtures go in `fixtures/import/qif/`: splits, transfers, multi-account, categories, investment, a Banktivity-style file (mirrored transfers, repeated per-currency lists), and a QMTF file.

**Phase 3: Banktivity round trip (manual, Patrick).**
1. Export from a fixture journal first, then his own.
2. Import into a Banktivity 10 trial.
3. Check accounts, categories, splits and transfers.
4. Export QIF from Banktivity.
5. Import it into a fresh folder with Ledgeline.
6. Compare `hledger bal` and `hledger is` against the original.

Record the findings in this plan. This also settles the open questions below.

**Phase 4: posting-row CSV, both ways.**
- **Export:** a CSV with columns compatible with hledger's `print -O csv` (`txnidx,date,date2,status,code,description,comment,account,amount,commodity,debit,credit,posting-status,posting-comment`). hledger 1.99.4 moved `debit` before `credit`; match whichever order the bundled version uses, or pick one and document it.
- **Import:** a generic grouped-CSV importer with presets for Xero Journal Report/GL Detail, GnuCash multi-line, **and hledger's own `print -O csv`**.
  - It must be **our own Rust importer** in `crates/ledgeline-core/src/grouped_csv.rs`, because hledger can't do the grouping (see §2).
  - Reuse the `convert/delimited.rs` reader. Group rows by key, by a key carried down to blank rows, or by a run that balances to zero.
  - Write journal text directly with `id:` tags, modelled on `qb_journal.rs` and `qb_import.rs`. No hledger subprocess, and no dependency on the bundled hledger version.
  - Build synthetic Xero fixtures from the documented columns (Journal ID, Date, Account, Account Code, Description, Debit, Credit). Patrick's old Xero exports are **not needed**. At most, ask for the header row only (no data) to confirm the column names, and only if the documented columns turn out to be wrong.
  - Xero edits create a reversal journal plus a new one. Import both by default; netting them out is optional.

**Phase 5: single-entry presets.**
- Monarch transactions and balance-history CSVs, YNAB, Copilot.
- Use the existing rules generation (`crates/ledgeline-core/src/rules/generate.rs`) and transfer matching (`matching.rs`, `docs/journal/transfer-matching.md`).
- YNAB splits: group consecutive rows by the `Split (n/N)` memo prefix. Transfers show as Payee `Transfer : X` in both accounts; dedupe them.

**Phase 6: File → New wizard.**
- Menu item, then an empty-folder picker, then the questions, then either a template chart of accounts or "import from…" (phases 2, 4 and 5).
- Replace the `fixtures/sample.journal` fallback at first run (`resolve_journal` / `DEFAULT_FIXTURE` in `main.rs`) with the wizard.
- Reuse the chart-of-accounts research in `plans/19-account-balances.md` (around line 81: 108 public plain-text-accounting repos plus the GnuCash, QuickBooks and Xero default charts).
- Reuse the "Create accounts.journal" code from plan 18 (`plans/18-account-list-editor.md`, `docs/settings/accounts.md`).
- Account tags the wizard can seed: `type:` (A/L/E/R/X/C/V/G), `issection`, `isgroup`, `bsgroup`, `bsterm`, `cfsection`, `cfgroup`, `holdings`, `valuation`, `kpi`, `clearing:`.

## Verification

- **Unit tests:** QIF writer golden files per fixture. A property test for export → import equivalence on the supported subset (per-account balances equal before and after). Loss-report tests for multi-currency, holding-account and investment cases.
- **Integration:** run `just test-integration` after any `/api` change, because `test:unit` skips the live-engine tests. Run the CLI round trip on `fixtures/*.journal`.
- **Manual:** the Phase 3 Banktivity round trip, with `hledger bal --tree` diffs recorded.

## Open questions (settle with sample files or the Phase 3 trial)

- Does Banktivity's QIF import create categories from `!Type:Cat`, or only from the `L` fields it sees? The docs don't say.
- Does Banktivity double-count a transfer written once with `L[Account]`, or one written in both accounts? Pick whichever it handles cleanly.
- What does Banktivity's QIF export contain for investments, budgets and schedules? Probably nothing for budgets or schedules.
- Do Monarch's and Copilot's CSVs include split rows or IDs? This needs an export containing a known split.
- How does a split look in Quicken Mac's CSV export?
- Does Xero's Journal Report export include a Source column?

---

## Research appendix (sources; don't redo)

### Banktivity
- Current version: Banktivity 10, released 2026-05-26. What's new: https://www.banktivity.com/content/features/whats_new.php
- Overview and tiers: https://www.banktivity.com/content/personal-finance-software-mac/ and https://www.banktivity.com/content/compare/
- 9.5 release (monthly plans, Plaid): https://www.iggsoftware.com/blog/2024/10/banktivity-9-5-and-monthly-subscriptions/
- Import help:
  - https://www.banktivity.com/content/help/v10/importing/about.php
  - https://www.banktivity.com/content/help/v10/importing/file_import.php
    - multi-account QIF: you pick a destination per account;
    - transfers auto-matched;
    - duplicates screened;
    - ambiguous dates prompt the user;
    - imported transactions marked cleared;
    - reconciliations are not imported.
  - https://www.banktivity.com/content/help/v9/getting_started/new_data.php
- Export help (QIF/TXF/CSV; QIF can be all or selected accounts over a date range, with optional redaction; one file per currency): https://www.banktivity.com/content/help/v10/export.php
- QMTF is a QIF variant; renaming it to `.qif` is IGG's workaround: https://www.banktivity.com/support/articles/banktivity-for-mac/why-cant-i-import-my-qmtf-file-from-quicken-mac/
- Moving from Quicken Windows:
  - keeps banking transactions, most investment transactions, categories, classes and securities;
  - loses scheduled and memorized transactions, budgets, reports, price history, ESPP and loans.
  - https://www.banktivity.com/support/articles/banktivity-6/how-do-i-move-from-quicken-windows-to-banktivity-6-for-mac/
- Complaints:
  - Banktivity QIF into Quicken duplicates transfers, and split transfers break: https://community.quicken.com/discussion/7949080/issues-converting-from-banktivity-7
  - Per-currency QIFs repeat the account and category lists: https://infinitekind.tenderapp.com/discussions/switching-from-another-personal-finance-program/18737-import-from-banktivity-multiple-currency-accounts
- `.bank8` internals (Core Data SQLite; writing to it directly breaks sync): https://github.com/rjeans/banktivity-mcp and https://github.com/david00769/banktivity-swift-mcp
- Third-party review: https://machow2.com/banktivity-review/

### QIF and OFX formats
- QIF spec:
  - https://www.w3.org/2000/10/swap/pim/qif-doc/QIF-doc.htm
  - https://en.wikipedia.org/wiki/Quicken_Interchange_Format
  - GnuCash notes on the format's ambiguities: https://stuff.mit.edu/afs/sipb/project/gnucash-test/src/gnucash/src/import-export/qif-import/file-format.txt
- QIF essentials:
  - Headers: `!Type:Bank|Cash|CCard|Invst|Oth A|Oth L`, plus `!Account`, `!Type:Cat` and `!Type:Class` lists.
  - Each transaction ends with `^`. Fields: `D` date, `T` total, `P` payee, `M` memo, `N` number or investment action, `C` status (blank, `*`/`c` cleared, `X`/`R` reconciled), `L` category or `[Account]` transfer.
  - Splits are repeated `S` (category or `[Account]`), `E` (memo), `$` (amount) and `%` lines.
  - Investment actions: Buy, Sell, Div, ReinvDiv, StkSplit, ShrsIn/Out. A trailing `X` means the cash goes to the `L` account.
  - No currency field and no transaction IDs.
- OFX: a single-account statement with signed amounts and FITID; no categories, splits or transfer links. Python `ofxtools` can generate OFX if a reference writer is needed: https://ofxtools.readthedocs.io/en/latest/generating.html
- Rust crates:
  - `qif` reads and writes but is immature (v0.1): https://crates.io/crates/qif
  - `qif_parser` is read-only (splits, investments, serde) and is a useful reference: https://crates.io/crates/qif_parser
  - `ofxy` is parse-only and GPL-3.0, so avoid it as a dependency.
  - No Rust crate writes OFX. `ofx`/`ofx-rs` are OpenFX (visual effects), not financial OFX.
  - Decision: hand-write the QIF writer and parser.
- No mature QIF writer exists for ledger, hledger or beancount; every tool goes the other direction. qif2ledger (https://github.com/chrisnatali/ledger-tools) and ledgerbridge (https://github.com/MugenLab/ledgerbridge) are references for import.

### Other apps' exports
- **Quicken:**
  - Windows QIF export: File > File Export > QIF.
  - Mac has had no QIF export since 2007: https://community.quicken.com/discussion/7963812/quicken-2017-on-a-mac-no-longer-has-qif-as-an-export-option
  - Mac CSV export: https://info.quicken.com/mac/exporting-register-transactions-to-a-csv-spreadshe
  - Windows Excel/CSV export puts splits on continuation rows with blank Date/Account/Num/Description: https://community.quicken.com/discussion/7868904/
  - QIF export guide: https://qifqbo.com/blog/how-to-export-qif-from-quicken
- **Monarch:**
  - Help: https://help.monarch.com/hc/en-us/articles/15526600975764-Downloading-Transaction-or-Account-History (returned 403 to automated fetches).
  - Columns: `Date,Merchant,Category,Account,Original Statement,Notes,Amount,Tags`, sometimes with `Owner`.
  - The balance-history CSV has date, balance and account.
  - Migration guides: https://www.cadencemoney.com/support/switch-from-monarch and https://summitward.com/learn/import-monarch-money-data
- **Xero:**
  - Journal Report: one row per journal line, with Journal ID, Date, Posted Date, Account, Account Code, Description, Debit and Credit. https://help.xero.com/au/Report_Journal
  - GL Detail can add Journal ID, Source and Reference: https://support.validis.com/hc/en-gb/articles/37948517395860-Downloading-GL-TB-from-Xero and https://support.mindbridge.ai/hc/en-us/articles/5873992135447-Xero-Export-the-general-ledger
  - Patrick's links (JavaScript-rendered, so not fetchable): https://central.xero.com/0/article/Export-data-out-of-Xero-US and https://central.xero.com/0/article/Export-general-ledger-data-out-of-Xero?userregion=true
- **MYE (MYOB):**
  - https://practice-support.myob.com/supps/importing-mye-files-into-general-ledger
  - https://practice-support.myob.com/cah/importing-data
  - https://community.myob.com:443/t5/Accountants-Office-Accountants/Export-mye-from-Xero-into-AE-Client-Accounting/m-p/440857
- **YNAB:**
  - The export is a zip with Plan and Register CSVs.
  - Register columns: `Account,Flag,Date,Payee,Category Group/Category,Category Group,Category,Memo,Outflow,Inflow,Cleared`.
  - https://www.npmjs.com/package/parse-ynab-export and https://arcabudget.app/guides/import-from-ynab-actual/
- **Lunch Money:** CSV with Transaction number, Type and Parent ID; groups are only in the API. https://support.lunchmoney.app/finances/transactions/other-features and https://lunchmoney.dev/v1/transactions
- **Actual Budget:** split rows marked by `(SPLIT INTO n)` and `(SPLIT x OF y)` note prefixes. https://github.com/actualbudget/actual/pull/2973 and https://github.com/actualbudget/actual/pull/9069
- **Copilot:** CSV with date, name, amount, type, category, account, account mask, excluded and notes. https://nethaven.app/blog/leaving-copilot-money-what-transfers
- **GnuCash:**
  - Multi-line CSV has one row per split, with the transaction ID on the first row only: https://lists.gnucash.org/docs/C/gnucash-manual/trans-export.html
  - The CSV is unreliable for currency; g2b reads SQLite instead: https://pypi.org/project/g2b
- **Moneydance:** QIF, tab-delimited and Raw JSON exports. https://infinitekind.tenderapp.com/kb/backups-and-exporting/exporting-data
- **QuickBooks Online:**
  - The Journal report has no transaction ID column. Group rows by date, type and number, and cut when debits equal credits.
  - Ledgeline already handles this in `qb_journal.rs`.
  - Reference: https://github.com/turbogizzmo/LocalBooks/pull/75

### hledger
- `print -O csv` columns: listed in Phase 4. `debit` moved before `credit` in 1.99.4 (2026-09-10).
- CSV rules support `account1..99` / `amount1..99` on a single row.
- `merge N` arrived in 1.99.5 (2026-10-01); it only joins a fixed N rows.
- The latest release is 1.99.6 (2026-11-01). Ledgeline's behaviour notes were verified on 1.52.
- References:
  - https://hledger.org/manual.html
  - https://hledger.org/relnotes.html
  - https://forum.plaintextaccounting.org/t/is-it-possible-to-combine-2-csv-transactions-into-1-using-hledger/635

### Existing Ledgeline code to reuse (inventory 2026-11-12)

**Import pipeline A** (statements → CSV → hledger rules):
- `crates/ledgeline-core/src/convert/` (`mod.rs` `SourceFormat`, `delimited.rs`, `ofx.rs`, `spreadsheet.rs`, `encoding.rs`).
- Rules: `rules.rs` and `rules/`. Numbered `accountN` fields are parsed, but the editor only exposes `account1`/`account2`.
- `review.rs` (category suggestions) and `reimport.rs` (FITID → `id:` dedupe).
- `matching.rs` (transfers).
- OFX `INVSTMTRS` is refused.

**Import pipeline B** (QuickBooks Online Journal xlsx): `qb_journal.rs`, `qb_import.rs`, `crates/ledgeline-server/src/qb_journal_api.rs`, `docs/imports/quickbooks.md`, `plans/17-quickbooks-journal-import.md`. It handles full multi-posting, alias mapping, `id:` dedupe and `--update-changed`. This is the template for QIF and grouped-CSV import.

**Export:**
- Today it's xlsx only: `web/src/lib/export/journalXlsx.ts` (QBO-style journal), `web/src/lib/export/xlsx.ts` (reports), `web/src/lib/projections/export.ts`.
- There is no CSV, QIF, OFX or journal-file export.

**Journal model:**
- `parse.rs`: native parser; skips `=` auto-posting blocks when parsing.
- `auto.rs`: applies auto-postings the way `hledger print --auto` does.
- `reports/account_types.rs`: type resolution; see `fixtures/account-types/non-english.journal`.

**Settings:**
- Code: `crates/ledgeline-server/src/prefs.rs`, `ui_prefs.rs`, `recents.rs`. Docs: `docs/settings/settings-files.md`.
- Files: `prefs.toml`, `ui-prefs.toml`, `ledgeline.toml` (never auto-created) and `recent.json`. The config folder can be overridden with `LEDGELINE_CONFIG_DIR`.

**Fixtures:**
- `fixtures/import/{delimited,spreadsheet,ofx,qb-journal,match,sort,layouts,reimport}`. There are no QIF or other-app fixtures yet.
- A global `*-journal` gitignore hides files in `fixtures/import/qb-journal/`. Watch for similar traps and use `git add -f`.

---

## Implementation notes (2026-11-12)

### Architecture

Every whole-book importer parses into the QuickBooks journal import's existing shape (`QbJournal` → `QbTransaction` → `QbPosting` in `qb_journal.rs`) and reuses its stage, alias-mapping preview, `id:` dedupe and commit, rather than adding parallel pipelines. `book_import::detect_and_parse` is the single dispatch: QuickBooks, then QIF, then the posting-row CSV presets, then the single-entry presets; anything unclaimed goes on to the statement/rules pipeline. Shared fields added for this: `QbTransaction.status` (`ImportStatus`), `QbJournal.source` (`ImportSource`) and `QbJournal.account_kinds` (`SourceAccountKind`). Internal type names still say `Qb`.

### What was built

| Phase | Code | Docs |
|-------|------|------|
| 1 QIF export | `export/qif.rs`, `export/loss.rs`, `ledgeline export --qif`, File → Export → QIF… (`gui_export.rs`) | `docs/exports/qif.md` |
| 2 QIF import | `qif.rs`, `qif_import.rs`, `book_import.rs` | `docs/imports/qif.md` |
| 4 posting-row CSV | export `export/csv.rs` (`ledgeline export --csv`, File → Export → CSV); import `grouped_csv.rs` (hledger, Xero, GnuCash) | `docs/exports/csv.md`, `docs/imports/other-apps.md` |
| 5 single-entry | `single_entry.rs` (YNAB, Monarch, Copilot) | `docs/imports/other-apps.md` |
| 6 File → New | `new_ledger.rs`, `new_ledger_api.rs`, `web/src/lib/onboarding/`, File → New… (Cmd+N), first run opens the wizard | `docs/new-ledger.md` |

### Decisions and deviations

- **Phase 5 does not use rules generation.** Single-entry exports go through the same multi-posting mapping step as QIF: the account column is the tracked account, the category column the counter-account, and transfer rows are paired into one transaction. One reviewed step maps categories and pairs transfers, which suits a whole-book migration better than a rules file.
- **hledger 1.52 writes `credit` before `debit`** in `print -O csv`, not the order the Phase 4 text lists. The export matches 1.52 byte for byte (all 120 fixture journals hledger accepts; opt-in check in `just hledger-checks`). The importer reads columns by name, so either order imports.
- **The account tree is derived and suggested, not written.** The importers record each source account's kind (QIF register types and income categories; single-entry tracked accounts vs. categories, classified by the sign of their money). `qb_import::suggest_accounts` turns that into pre-filled mapping rows: an existing account with the same last segment if there is exactly one, otherwise a name under a standard root (`assets:bank:…`, `liabilities:credit cards:…`, `expenses:…`). GnuCash and hledger names are suggested as written. The user reviews every row before any alias is saved.
- **QIF transfers are written once** (in the first bank-like register by name). `--mirror-transfers` writes Quicken-style mirrored records for Phase 3 to compare. Status: `*` → `CX`, `!` → `C*`.
- **Virtual postings** are written by the CSV export (as hledger does) and left out, with a count, by the CSV importer. QIF has no form for them; the loss report counts them.
- **No fiscal-year question.** The plan listed one, but neither hledger nor Ledgeline has a fiscal-year setting, so the answer would have been unused. Removed 2026-11-12; add it back with the reports that would use it.
- **First run** opens the wizard in the GUI when there is no journal argument, `$LEDGELINE_FIXTURE` or recent journal. `--server` and the dev shell still fall back to `fixtures/sample.journal`. The wizard's "Try with sample data" writes a fresh copy of the embedded sample to the cache folder.

### Known gaps

- **Multi-currency import.** `QbPosting` has no commodity, so QIF amounts all go into the journal's default commodity (a second-currency Banktivity file imports wrongly) and the CSV presets refuse a file mixing commodities.
- **Ambiguous QIF dates** warn but don't yet let the user pick the other order.
- **Investments** export and import as cash movement; no `!Type:Invst` output, no lots.
- **Unverified real-world formats:** Xero's actual Journal Report layout and column names, GnuCash's multi-line first row and localized headers, Copilot's `type` values and sign, Monarch's transfer category names. Synthetic fixtures follow the documented columns.
- **Not exercised by hand:** the native folder/save dialogs, File → Export, File → New and the first-run window were build- and test-checked only.
