# QIF and QMTF fixtures

Corpus for `crates/ledgeline-core/src/qif.rs` (the reader) and `qif_import.rs` (QIF to the
multi-posting import shape). See `plans/39-onboarding-offboarding.md` Phase 2 and
`docs/imports/qif.md`.

Everything here is synthetic: the shapes follow the QIF spec and the exporters' documented
habits, and the payees, accounts and amounts are made up.

| File | What it exercises |
| --- | --- |
| `splits.qif` | A single-account file with no `!Account` record: the account name comes from its `Opening Balance` line. Splits with memos, a split income record (salary less withholding), a check number. |
| `transfers.qif` | One account's register with transfers to accounts not in the file (`L[Savings]`, `L[Visa]`), which become source names to map. |
| `multi-account.qif` | An `!Option:AutoSwitch` account list, then one register per account type (`Bank`, `CCard`, `Cash`, `Oth A`, `Oth L`), a split transfer to a loan, and opening balances. |
| `categories-classes.qif` | `!Type:Cat` and `!Type:Class` lists, and classes on `L` and `S` lines (`LRent/Duplex A`). |
| `investment.qif` | `!Type:Invst` actions (`XIn`, `Buy`, `Div`, `ReinvDiv`, `SellX`, `StkSplit`, `IntInc`) plus the bank side of the two cash transfers, which collapse into the investment records. |
| `banktivity.qif` | Banktivity's habits: every transfer written in both accounts (plain, card payment, and a split transfer whose other side is a plain record), and the account and category lists repeated, as in its per-currency files. Ten transactions survive. |
| `quicken-mac.qmtf` | Quicken for Mac's QMTF: an unknown `!Type:Tag` section, `U` alongside `T`, and Quicken's `1/ 2'26` dates. |
| `ambiguous-dates.qif` | Every date reads as both month/day and day/month, so the date format is flagged ambiguous. |
| `windows-1252.qif` | A payee in Windows-1252 (`Caf\xE9`), not UTF-8. |
