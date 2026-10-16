# Screenshot manifest

Every image `just docs-screenshots` writes, and where it belongs in the manual
(target structure from `plans/25-published-docs.md`). Paths are relative to the
repo root. All are 2× PNGs; the app has a single dark theme.

| Image                                              | What it shows                                                                                                | Doc page / section                            |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | --------------------------------------------- |
| `docs/journal/images/overview.png`                 | Journal view, last 90 days, Insights collapsed: filter bar, table, totals footer                             | `journal/index.md`, top                       |
| `docs/journal/images/filter-bar.png`               | Filter bar excerpt with an `expenses` filter: preset and date range boxed, Accounts picker circled, chip row | `journal/filtering.md`, accounts and dates    |
| `docs/journal/images/account-picker.png`           | Accounts dropdown open: search field and account tree                                                        | `journal/filtering.md`, accounts              |
| `docs/journal/images/transaction-editor.png`       | Edit transaction dialog for a four-posting paycheck                                                          | `journal/index.md`, editing transactions      |
| `docs/journal/images/problems-badge.png`           | Navbar excerpt, the problems badge (6) circled                                                               | `journal/problems.md`, top                    |
| `docs/journal/images/problems-drawer.png`          | Problems drawer open over the journal, all six findings                                                      | `journal/problems.md`, the rules              |
| `docs/journal/images/export-button.png`            | Totals footer, arrow to the Export .xlsx button                                                              | `journal/export.md`                           |
| `docs/imports/images/new-transactions.png`         | New Transactions tab: the drop zone                                                                          | `imports/new-transactions.md`, top            |
| `docs/imports/images/quickbooks-preview.png`       | QuickBooks Journal export preview: export instructions, counts, first sample transactions                    | `imports/quickbooks.md`, import               |
| `docs/imports/images/quickbooks-mapping.png`       | Map QuickBooks accounts table, first three rows, the "Your account" field circled                            | `imports/quickbooks.md`, accounts and mapping |
| `docs/imports/images/quickbooks-import-button.png` | Import button disabled with "Map every account above before importing." boxed                                | `imports/quickbooks.md`, accounts and mapping |
| `docs/imports/images/rules-editor.png`             | Edit Rules tab: rules file list, Preferences panel, first rules                                              | `imports/rules-editor.md`, top                |
| `docs/reports/images/insights.png`                 | Reports → Insights dashboard (year over year)                                                                | `reports/insights.md`, top                    |
| `docs/reports/images/balance-sheet.png`            | Balance Sheet as of 2026-07-08: unvalued-holdings warning, current/non-current assets                        | `reports/balance-sheet.md`, top               |
| `docs/reports/images/subscriptions.png`            | Subscriptions tab (subscriptions fixture): annual and monthly boxes, tagging help                            | `reports/subscriptions.md`, top               |
| `docs/holdings/images/stocks.png`                  | Holdings → Stocks: filter bar, Update prices, Insights summary and chart                                     | `holdings/` stocks page, top                  |
| `docs/holdings/images/stocks-table.png`            | Stocks holdings table excerpt: basis, price, market value, gain                                              | `holdings/` stocks page, the table            |
| `docs/holdings/images/update-prices.png`           | Arrow to the Update prices button                                                                            | `holdings/` stocks page, fetching prices      |
| `docs/holdings/images/other.png`                   | Holdings → Other: value-over-time chart, home and car rows                                                   | `holdings/` other-holdings page               |
| `docs/images/budget-overview.png`                  | Budget tab (budget fixture), year to date: controls, bars, goals heading                                     | `budget.md`, top                              |
| `docs/images/budget-bars.png`                      | Budget bars excerpt: Expenses roll-up and per-account bars, one "spent / budgeted" figure boxed              | `budget.md`, reading the bars                 |
| `docs/images/budget-goals.png`                     | Budget goals list: monthly goals with Edit/Remove                                                            | `budget.md`, editing goals                    |
| `docs/projections/images/seeded.png`               | Projections seeded from the journal: income and expense lines                                                | `projections/` index, top                     |
| `docs/projections/images/asset-scenario.png`       | asset-growth-scenario loaded: flows plus three asset rows, one with a balance override                       | `projections/` asset rows page, top           |
| `docs/projections/images/asset-rows.png`           | Assets and balances excerpt, the first asset row boxed (growth 7%/yr, $0 contribution)                       | `projections/` asset rows page, the row       |
| `docs/settings/images/accounts.png`                | Settings → Accounts with `assets:bank:checking` open in the editor                                           | `settings/accounts.md`, top                   |
| `docs/settings/images/account-aliases.png`         | Settings → Account Aliases: two plain aliases and one regex alias                                            | `settings/account-aliases.md`, top            |

## Not captured yet (plan 24)

Stubs are in `reports.docs.ts` as `test.skip`, waiting for the Interval/Total rework:

| Image                                              | What it should show                                         | Doc page / section                       |
| -------------------------------------------------- | ----------------------------------------------------------- | ---------------------------------------- |
| `docs/reports/images/income-statement.png`         | P&L, Interval dropdown circled, % change column visible     | `reports/income-statement.md`            |
| `docs/reports/images/income-statement-monthly.png` | P&L at Monthly, Total column boxed                          | `reports/income-statement.md`, intervals |
| `docs/reports/images/cash-flow.png`                | Cash Flow, From/To and Interval circled, Total column boxed | `reports/cash-flow-and-net-worth.md`     |
