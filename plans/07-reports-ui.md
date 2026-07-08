# WP-07: Reports UI + Excel Export

Read `plans/00-overview.md` first, then WP-06 contracts (report types + functions).

## Scope

The `/reports` route: report tabs, per-report controls, spreadsheet-style tables, and xlsx export. Plain tables only — no charts here (MVP requirement).

## Out of scope

Report computation (WP-06), budget report (post-MVP).

## Behavior

- **Tabs** (daisyUI `tabs`): Balance Sheet | P&L | Cash Flow | Net Worth. Active tab + controls in URL query params (same replaceState pattern as WP-04's urlSync).
- **Controls per report** (`ReportControls.svelte`, driven by a config object):
  - Balance sheet: as-of date (default today), depth slider
  - P&L: from/to (default this year), depth
  - Cash flow: end date (default today), interval monthly/quarterly/yearly, lookback count (default 12), depth
  - Net worth: end date, interval, lookback count
- **Tables** (`ReportTable.svelte`): sticky header + sticky account column, daisyUI zebra, indented account names by depth with single-child chain compression as display concern (`assets:bank:checking` when intermediate levels have no siblings), right-aligned formatted amounts (`formatAmount`), section subtotals + grand total rows emphasized, negatives in `text-error`. Handles both `SectionedReport` and `PeriodReport` (bucket columns; horizontal scroll on mobile).
- **Export** (`ExportButton.svelte` → `web/src/lib/export/xlsx.ts`):
  ```ts
  export async function exportXlsx(report: SectionedReport | PeriodReport, meta: { title: string; params: string }, filename: string): Promise<void>;
  ```
  exceljs via `await import("exceljs")` (never in the main bundle). One worksheet; header row styled; numbers written as JS numbers with Excel number format derived from decimal places (display-only conversion is acceptable at the export boundary — note in code comment is NOT needed, it's documented here). Trigger browser download (Blob + anchor).

## Key files created

`web/src/routes/reports/+page.svelte`, `web/src/lib/reports/ui/{ReportTabs,ReportControls,ReportTable,ExportButton}.svelte`, `web/src/lib/export/xlsx.ts`

## Depends on / parallel

Depends on: WP-06 (and WP-01 shell). Parallel with: WP-03, WP-04, WP-05, WP-08.

## Definition of done

- Against fixture journal: all four reports render; balance sheet numbers spot-match `hledger -f fixtures/sample.journal bs` at same depth/date
- Depth/interval/date controls recompute instantly (derived, no refetch); tab + params survive reload via URL
- Exported .xlsx opens in Excel/Numbers with correct values, number formats, and title row; exceljs chunk absent from initial bundle (`vite build` chunk check)
- Usable at 375px (horizontal-scroll tables) and desktop; `just check` green
- Commit: `feat: reports ui with xlsx export`
