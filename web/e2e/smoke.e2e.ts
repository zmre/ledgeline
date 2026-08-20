// E2E smoke against the REAL stack (WP-09): hledger-web serving
// fixtures/sample.journal on :5099 (see playwright.config.ts webServer) with
// the built SPA in front of it.
//
// Fixture facts, verified against hledger 1.52 CLI (see plans/09), and
// re-verified after WP-14 added the home, the car, the mortgage and the
// depreciation entries to sample.journal — every figure below INCLUDES them:
//   - 189 transactions total; 28 fall in the last-90 window 2026-04-10..2026-07-08
//     at the pinned clock (journal spans 2024-07-01..2026-07-04)
//   - `hledger bal expenses -b 2026-04-10 -e 2026-07-09` → $15,026.62 (+ 228,75 EUR);
//     the footer shows the negated primary-commodity net, $-15,026.62 (the window
//     contains the 2026-06-30 $3,500.00 vehicle depreciation)
//   - deepest account is 4 segments (assets:broker:taxable:vti) → depth-slider max 4
//     (the slider is on cf/nw/budget only; BOTH statements dropped it and ask the
//     engine for an unclamped report)
//   - `hledger is -V -b 2026-01-01 -e 2027-01-01 --depth 2`: the P&L tab is
//     GROUPED and market-valued since plans/13, and its default range is the
//     calendar year — Revenue $34,010.00, Expenses $28,626.48, Net $5,383.52.
//     Expenses INCLUDE depreciation: `expenses:depreciation` is a declared
//     expense account, so the engine's P&L counts the write-down ($3,500.00 in
//     2026, $4,000.00 in 2025). A figure $3,500/$4,000 lower is the stale
//     pre-WP-14 number — do not "correct" the assertions to it. Its prior
//     column is the previous equal-length window, which for a full calendar
//     year is the previous calendar year: 2025 reads $66,428.75 / $48,450.54 /
//     $17,978.21.
//   - `hledger bs -V -e 2026-07-09` (CLI -e is exclusive ≙ our asOf
//     2026-07-08): the Balance Sheet tab is MARKET-valued since plans/12, so
//     Total Assets $548,112.615 (+ 5 GLD − 2 TSLA, both unpriced), Liabilities
//     $336,531.15, net worth $211,581.465 — the WP-14 home ($468,000.00), car
//     ($20,500.00 net of accumulated depreciation) and $336,000.00 mortgage
//     all included. Unvalued (`hledger bs`) the dollar column alone reads
//     $68,902.56, with the stocks, EUR and HOME left as raw commodity balances
//     — that is the OLD tab, and the numbers below are deliberately not those.
//   - 6 deliberate problem records: pending txn, expenses:unknown, empty description,
//     GLD missing basis, GLD unpriced, TSLA negative shares (WP-10)
//
// The clock is pinned to 2026-07-08 so the last-90 default preset, the reports
// default as-of date, and the future-date check stay glued to those facts.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW); // Date is fake, timers keep running (URL-sync debounce, polling)
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

test("journal: last-90 default preset filters, all-time shows the full journal", async ({page}) => {
    await page.goto("/");

    // Default preset is "last 90 days" (defaultFilter → last90); at the pinned
    // clock that's 2026-04-10 … 2026-07-08. The table is virtualized (row count
    // is viewport-bound), so the TotalsFooter is the source of truth for counts.
    const footer = page.locator("footer");
    await expect(footer).toContainText("28 transactions");
    await expect(footer).toContainText("2026-04-10 – 2026-07-08");
    await expect(page.locator("tbody tr").first()).toBeVisible();

    await page.locator("summary").filter({hasText: "Last 90 days"}).click();
    await page.getByRole("button", {name: "All time"}).click();
    await expect(footer).toContainText("189 transactions");
    await expect(footer).toContainText("all dates");
});

test("journal: selecting the expenses subtree nets the totals footer", async ({page}) => {
    await page.goto("/");

    const footer = page.locator("footer");
    await expect(footer).toContainText("28 transactions"); // journal loaded, last-90 filter active

    await page.locator("summary").filter({hasText: "Accounts"}).click();
    await page.getByRole("checkbox", {name: "expenses", exact: true}).check();

    // Visible Journal Total = net of the selected expenses postings over the
    // last-90 window, shown negative (money spent). The footer reports the primary
    // (most-used) commodity only — $ here — verified vs
    // `hledger bal expenses -b 2026-04-10 -e 2026-07-09` ($15,026.62 — it includes
    // the 2026-06-30 vehicle depreciation).
    await expect(footer).toContainText("$-15,026.62");
});

test("journal: insights depth slider starts at the default, not browser-clamped (regression)", async ({page}) => {
    await page.goto("/");

    // The insights panel mounts before the journal finishes loading, when the max
    // account depth is still 1; the browser clamps the range input to that max.
    // Once the real accounts arrive (fixture's deepest is 4) the slider must
    // re-apply its default (2) rather than stay stuck at 1.
    const slider = page.locator('input[aria-label="Account depth"]');
    await expect(slider).toHaveAttribute("max", "4"); // real max loaded, not the initial 1
    await expect(slider).toHaveValue("2"); // default depth, matching the chart
});

test("reports: balance sheet shows known fixture numbers", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Reports"}).click();
    // Reports opens on the Insights dashboard, so select the balance sheet first.
    await page.getByRole("tab", {name: "Balance Sheet"}).click();

    // Default balance-sheet params with the pinned clock: asOf 2026-07-08, no
    // depth clamp, valued at MARKET (plans/12) — so these are `hledger bs -V`'s
    // numbers, not `hledger bs`'s. Verified against hledger 1.52:
    //   bal assets -V -e 2026-07-09 → $548112.615, 5.0 GLD, -2.0 TSLA
    //   liabilities                 → $336531.15
    // The six figures are the WP-14 home ($468,000.00 at its 2026-06-30 price)
    // and car ($20,500.00), against the $336,000.00 mortgage that bought them.
    // Scoped to each box: "Total Assets" is deliberately repeated in the summary
    // and the tie-out below, so an unscoped locator is a strict-mode violation.
    await expect(page.getByTestId("bs-section-assets").locator("tr", {has: page.locator('th:text-is("Total Assets")')})).toContainText("$548,112.62");
    await expect(page.getByTestId("bs-section-liabilities").locator("tr", {has: page.locator('th:text-is("Total Liabilities")')})).toContainText("$336,531.15");

    // The tie-out: liabilities + equity must come back to total assets, and that
    // pair — not net worth — carries the verdict.
    const tieOut = page.getByTestId("bs-tie-out");
    await expect(page.getByTestId("bs-summary")).toContainText("Liabilities + Equity");
    await expect(tieOut).toContainText("$548,112.62");
    await expect(tieOut).toContainText("Balanced");

    // Net worth = assets − liabilities = $211,581.465 exactly. We show
    // $211,581.47: `formatDec` rounds half AWAY FROM ZERO everywhere in this app,
    // while hledger's CLI prints $211,581.46 because Haskell's `round` is
    // half-to-even. The cent is a display convention, not a disagreement.
    await expect(page.getByTestId("bs-net-worth")).toContainText("$211,581.47");

    // GLD and TSLA have no `P` directive, so the valuation could not convert
    // them. They must be visible, never silently dropped from the total.
    await expect(page.getByTestId("unpriced-warning")).toContainText("GLD");
    await expect(page.getByTestId("unpriced-warning")).toContainText("TSLA");

    // The journal balances, so the check line stays off the screen entirely.
    await expect(page.getByTestId("bs-check")).toHaveCount(0);

    // No depth slider on this tab: groups are the reading and the accounts under
    // one are a drill-down, so the clamp had nothing left to say. It is still on
    // the tabs whose tables it does move — but the P&L is no longer one of them
    // (plans/13), so Cash Flow is what proves the control still exists at all.
    const depthSlider = page.locator('input[aria-label="Account depth"]');
    await expect(depthSlider).toHaveCount(0);
    await page.getByRole("tab", {name: "P&L"}).click();
    await expect(depthSlider).toHaveCount(0);
    await page.getByRole("tab", {name: "Cash Flow"}).click();
    await expect(depthSlider).toHaveCount(1);
});

test("reports: P&L shows grouped boxes with known fixture numbers", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Reports"}).click();
    await page.getByRole("tab", {name: "P&L"}).click();

    // Default P&L params with the pinned clock: the calendar year 2026, valued at
    // MARKET (plans/13). Verified against hledger 1.52,
    // `is -V -b 2026-01-01 -e 2027-01-01 --depth 2`.
    const revenue = page.getByTestId("is-section-revenue");
    const expenses = page.getByTestId("is-section-opex");
    await expect(revenue.locator("tfoot")).toContainText("$34,010.00");
    await expect(expenses.locator("tfoot")).toContainText("$28,626.48");

    // The complaint this redesign fixes: $34,010.00 appeared as an `income`
    // roll-up row, then again under each account, then again as "Total
    // Revenues". The section total now lives ONLY in the footer — the body
    // holds group lines, and no group is the whole section.
    await expect(revenue.locator("tbody")).not.toContainText("$34,010.00");
    await expect(revenue.locator("tbody")).toContainText("$33,960.00"); // Salary, the group line

    // The adaptive shape: an untagged personal journal earns no rung of the
    // GAAP ladder and no empty boxes.
    await expect(page.getByTestId("is-section-cogs")).toHaveCount(0);
    await expect(page.getByTestId("is-subtotal-grossProfit")).toHaveCount(0);
    await expect(page.getByTestId("is-subtotal-ebitda")).toHaveCount(0);
    // …and `opex` is titled plain "Expenses", not "Operating expenses".
    await expect(expenses.getByRole("heading", {name: "Expenses"})).toBeVisible();

    // The prior column: the previous equal-length window, which for a full
    // calendar year is the previous calendar year (hledger `-b 2025-01-01
    // -e 2026-01-01` → $66,428.75 / $48,450.54).
    await expect(revenue.locator("tfoot")).toContainText("$66,428.75");
    await expect(expenses.locator("tfoot")).toContainText("$48,450.54");

    // % of revenue, from the exact Decs: 28,626.48 / 34,010.00 = 84.1707% → 84.2%.
    await expect(expenses.locator("tfoot")).toContainText("84.2%");
    await expect(revenue.locator("tfoot")).toContainText("100.0%");

    // The summary below the boxes is net income and NOTHING else. A condensed
    // "Total Revenue / Less: Expenses / …" table stood there and was removed:
    // every figure in it was already a box footer directly above, which is the
    // duplicate-total complaint this redesign exists to fix, one panel down.
    const net = page.getByTestId("is-net-income");
    await expect(net).toContainText("$5,383.52");
    await expect(net).toContainText("$17,978.21"); // the prior year's
    await expect(net).toContainText("15.8%");
    await expect(page.getByTestId("is-summary")).toHaveCount(0);
    // So each section total is on the page exactly once, in its own footer.
    await expect(page.getByTestId("income-statement").getByText("$28,626.48", {exact: true})).toHaveCount(1);
});

test("reports: P&L groups start collapsed and open to their accounts", async ({page}) => {
    // The range the plan's ground-truth table pins, rather than the default
    // calendar year, so the figures below are the ones hledger printed for it.
    // No `depth`: the tab has no such control any more and asks the engine for an
    // unclamped report, so the URL carries nothing about it.
    await page.goto("/reports?tab=is&from=2026-01-01&to=2026-07-08");

    const expenses = page.getByTestId("is-section-opex");
    const food = expenses.getByRole("button", {name: "Food"});

    // Collapsed by default: the group SUBTOTAL is the whole reading, and the
    // accounts behind it are not on the page at all.
    await expect(food).toHaveAttribute("aria-expanded", "false");
    await expect(expenses).toContainText("$1,654.38");
    await expect(page.locator('[data-account="expenses:food"]')).toHaveCount(0);
    await expect(page.locator('[data-account="expenses:food:groceries"]')).toHaveCount(0);

    // Expanded: the drill-down appears, and its parts still add to the subtotal
    // the collapsed row showed ($1,272.50 + $381.88 = $1,654.38).
    await food.click();
    await expect(food).toHaveAttribute("aria-expanded", "true");
    await expect(page.locator('[data-account="expenses:food:groceries"]')).toContainText("$1,272.50");
    await expect(page.locator('[data-account="expenses:food:restaurants"]')).toContainText("$381.88");

    // Full depth, and single-child chains folded exactly as every other report
    // table folds them: `expenses:housing` holds nothing itself, so the pair
    // renders as one `expenses:housing:rent` row.
    const housing = expenses.getByRole("button", {name: "Housing"});
    await housing.click();
    await expect(page.locator('[data-account="expenses:housing:rent"]')).toContainText("$13,125.00");
    await expect(page.locator('[data-account="expenses:housing"]')).toHaveCount(0);

    // A group that ran in only ONE window still appears, with an explicit zero on
    // the other side — the union join the engine does over section/group/account
    // keys. Nothing landed in `expenses:unknown` during 2025-06-26..2025-12-31.
    const unknownRow = page.locator('[data-is-row="opex/Unknown"]');
    await expect(unknownRow).toContainText("$75.00");
    await expect(unknownRow).toContainText("$0.00");
});

test("reports: balance-sheet groups start collapsed and open to their accounts", async ({page}) => {
    // No `depth`: the tab has no such control any more and asks the engine for an
    // unclamped report, so the URL carries nothing about it. (A bookmarked
    // `&depth=3` still loads — `searchToParams` is not tab-gated — it is just
    // ignored here and dropped from the address bar on the next mirror.)
    await page.goto("/reports?tab=bs&asof=2026-07-08");

    const assets = page.getByTestId("bs-section-assets");
    const cash = assets.getByRole("button", {name: "Cash and cash equivalents"});

    // Collapsed by default: the group SUBTOTAL is the whole reading, and the
    // accounts behind it are not on the page at all.
    await expect(cash).toHaveAttribute("aria-expanded", "false");
    await expect(assets).toContainText("$49,059.99");
    await expect(assets).not.toContainText("$42,450.24"); // the assets:bank drill-down
    await expect(page.locator('[data-account="assets:bank"]')).toHaveCount(0);
    await expect(page.locator('[data-account="assets:bank:checking"]')).toHaveCount(0);

    // Expanded: the drill-down appears, and its parts still add to the subtotal
    // the collapsed row showed ($28,292.81 + $13,500.00 + $657.43 = $42,450.24,
    // plus assets:broker:taxable:cash $6,609.75 = the group's $49,059.99).
    await cash.click();
    await expect(cash).toHaveAttribute("aria-expanded", "true");
    await expect(assets).toContainText("$42,450.24");
    await expect(page.locator('[data-account="assets:bank:checking"]')).toContainText("$28,292.81");

    // Full depth, so the Wise EUR account is reachable. It is FOUR segments
    // down — under the old depth-3 clamp the row said `assets:bank:wise` and
    // stood for an account there was no longer any control to ask for.
    // `assets:bank:wise` holds nothing itself, so `compressSectionRows` folds
    // the chain into a single `wise:eur` row carrying the child's account name.
    await expect(page.locator('[data-account="assets:bank:wise:eur"]')).toContainText("$657.43");
    await expect(page.locator('[data-account="assets:bank:wise"]')).toHaveCount(0);
});

// UNRUN in the sandbox this was written in — no browser could be launched — so
// every figure below comes from `fixtures/native/v1/balancesheet-grouped.json`,
// the bytes the engine actually produced for this journal, rather than from our
// own rendering.
//
// The ADAPTIVE half of this feature has no e2e: `fixtures/sample.journal` now
// carries `bsterm: noncurrent` on the home, the car and the mortgage, so the
// only journal this stack serves is a classified one. "A journal that tags
// nothing renders exactly what it rendered before" is pinned instead by
// `balanceSheetRows.test.ts` and `BalanceSheetView.svelte.test.ts`, which can
// hold both shapes side by side.
test("reports: balance sheet bands assets and liabilities into current and non-current", async ({page}) => {
    await page.goto("/reports?tab=bs&asof=2026-07-08");

    const assets = page.getByTestId("bs-section-assets");
    const current = page.getByTestId("bs-subsection-assets-current");

    // Both bands, current FIRST — the engine's ordering, which the row model
    // relies on to know where one band ends and the next begins.
    await expect(assets.locator('[data-testid^="bs-subsection-"]')).toHaveText(["Current", "Non-current"]);

    // Each band closes on its own subtotal, labelled by the ENGINE — note
    // "assets" against "liabilities" below, which is why the label is a wire
    // field and not something composed on this side.
    //   Current     = cash $49,059.99
    //   Non-current = investments $10,552.625 + home $468,000 + car $20,500
    //               = $499,052.625 → $499,052.63 (half away from zero)
    await expect(page.getByTestId("bs-subtotal-assets-current")).toContainText("Total current assets");
    await expect(page.getByTestId("bs-subtotal-assets-current")).toContainText("$49,059.99");
    await expect(page.getByTestId("bs-subtotal-assets-noncurrent")).toContainText("Total non-current assets");
    await expect(page.getByTestId("bs-subtotal-assets-noncurrent")).toContainText("$499,052.63");

    // The bands are parts; the box footer is still the whole, and still the
    // figure the tie-out ties to.
    await expect(assets.locator("tfoot")).toContainText("$548,112.62");

    // A subheading is a heading, not a line: the band's total is on the row that
    // closes it, and printing it twice would invite a hunt for the difference.
    await expect(current).toHaveText("Current");

    // Liabilities band on the same axis with their own prose: the $531.15 visa
    // balance is current, the $336,000.00 mortgage is not.
    await expect(page.getByTestId("bs-subtotal-liabilities-current")).toContainText("Total current liabilities");
    await expect(page.getByTestId("bs-subtotal-liabilities-current")).toContainText("$531.15");
    await expect(page.getByTestId("bs-subtotal-liabilities-noncurrent")).toContainText("$336,000.00");

    // Equity is never banded, in any journal.
    await expect(page.getByTestId("bs-subsection-equity-current")).toHaveCount(0);
    await expect(page.getByTestId("bs-subsection-equity-noncurrent")).toHaveCount(0);
    await expect(page.getByTestId("bs-subtotal-equity-noncurrent")).toHaveCount(0);
});

test("reports: the balance-sheet cursor walks past the band rows", async ({page}) => {
    // Bands are not cursorable: neither a subheading nor a subtotal can be
    // expanded or drilled into, so stopping on one would be a stop where Enter
    // does nothing. `j` from a fresh load must therefore land on the first GROUP
    // — Cash and cash equivalents — and not on the "Current" heading above it.
    await page.goto("/reports?tab=bs&asof=2026-07-08");
    await expect(page.getByTestId("bs-subsection-assets-current")).toBeVisible();

    await page.keyboard.press("j");
    await expect(page.locator("[aria-current='true']")).toContainText("Cash and cash equivalents");

    // The next stop is the first group of the NEXT band, walking over that
    // band's subtotal and its heading both.
    await page.keyboard.press("j");
    await expect(page.locator("[aria-current='true']")).toContainText("Investments");

    // And the disclosures still work inside a band.
    await page.getByTestId("bs-section-assets").getByRole("button", {name: "Property"}).click();
    await expect(page.locator('[data-account="assets:property:home"]')).toBeVisible();
});

test("problems badge shows the deliberate problem count", async ({page}) => {
    await page.goto("/");

    // WP-09's three deliberate problems + WP-10's three stock warnings
    // (no future-dated txns at the pinned clock).
    await expect(page.getByRole("button", {name: "6 problems"})).toBeVisible();
});
