// E2E smoke against the REAL stack (WP-09): hledger-web serving
// fixtures/sample.journal on :5099 (see playwright.config.ts webServer) with
// the built SPA in front of it.
//
// Fixture facts, verified against hledger 1.52 CLI (see plans/09):
//   - 185 transactions total; 27 fall in the last-90 window 2026-04-10..2026-07-08
//     at the pinned clock (journal spans 2024-07-01..2026-07-04)
//   - `hledger bal expenses -b 2026-04-10 -e 2026-07-09` → $11,526.62 (+ 228,75 EUR);
//     the footer shows the negated primary-commodity net, $-11,526.62
//   - deepest account is 4 segments (assets:broker:taxable:vti) → depth-slider max 4
//     (the slider is on is/cf/nw/budget only; the balance sheet dropped it and
//     asks the engine for an unclamped report)
//   - `hledger bs -V -e 2026-07-09` (CLI -e is exclusive ≙ our asOf
//     2026-07-08): the Balance Sheet tab is MARKET-valued since plans/12, so
//     Total Assets $59,612.615 (+ 5 GLD − 2 TSLA, both unpriced), Liabilities
//     $531.15, net worth $59,081.465. Unvalued (`hledger bs`) it would read
//     $48,402.56 / $47,871.41 — that is the OLD tab, and the numbers below are
//     deliberately not those.
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
    await expect(footer).toContainText("27 transactions");
    await expect(footer).toContainText("2026-04-10 – 2026-07-08");
    await expect(page.locator("tbody tr").first()).toBeVisible();

    await page.locator("summary").filter({hasText: "Last 90 days"}).click();
    await page.getByRole("button", {name: "All time"}).click();
    await expect(footer).toContainText("185 transactions");
    await expect(footer).toContainText("all dates");
});

test("journal: selecting the expenses subtree nets the totals footer", async ({page}) => {
    await page.goto("/");

    const footer = page.locator("footer");
    await expect(footer).toContainText("27 transactions"); // journal loaded, last-90 filter active

    await page.locator("summary").filter({hasText: "Accounts"}).click();
    await page.getByRole("checkbox", {name: "expenses", exact: true}).check();

    // Visible Journal Total = net of the selected expenses postings over the
    // last-90 window, shown negative (money spent). The footer reports the primary
    // (most-used) commodity only — $ here — verified vs
    // `hledger bal expenses -b 2026-04-10 -e 2026-07-09` ($11,526.62).
    await expect(footer).toContainText("$-11,526.62");
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
    //   bal assets -V -e 2026-07-09 → $59612.615, 5.0 GLD, -2.0 TSLA
    //   liabilities                 → $531.15
    // Scoped to each box: "Total Assets" is deliberately repeated in the summary
    // and the tie-out below, so an unscoped locator is a strict-mode violation.
    await expect(page.getByTestId("bs-section-assets").locator("tr", {has: page.locator('th:text-is("Total Assets")')})).toContainText("$59,612.62");
    await expect(page.getByTestId("bs-section-liabilities").locator("tr", {has: page.locator('th:text-is("Total Liabilities")')})).toContainText("$531.15");

    // The tie-out: liabilities + equity must come back to total assets, and that
    // pair — not net worth — carries the verdict.
    const tieOut = page.getByTestId("bs-tie-out");
    await expect(page.getByTestId("bs-summary")).toContainText("Liabilities + Equity");
    await expect(tieOut).toContainText("$59,612.62");
    await expect(tieOut).toContainText("Balanced");

    // Net worth = assets − liabilities = $59,081.465 exactly. We show $59,081.47:
    // `formatDec` rounds half AWAY FROM ZERO everywhere in this app, while
    // hledger's CLI prints $59,081.46 because Haskell's `round` is half-to-even.
    // The cent is a display convention, not a disagreement about the balance.
    await expect(page.getByTestId("bs-net-worth")).toContainText("$59,081.47");

    // GLD and TSLA have no `P` directive, so the valuation could not convert
    // them. They must be visible, never silently dropped from the total.
    await expect(page.getByTestId("unpriced-warning")).toContainText("GLD");
    await expect(page.getByTestId("unpriced-warning")).toContainText("TSLA");

    // The journal balances, so the check line stays off the screen entirely.
    await expect(page.getByTestId("bs-check")).toHaveCount(0);

    // No depth slider on this tab: groups are the reading and the accounts under
    // one are a drill-down, so the clamp had nothing left to say. It is still on
    // the tabs whose tables it does move.
    const depthSlider = page.locator('input[aria-label="Account depth"]');
    await expect(depthSlider).toHaveCount(0);
    await page.getByRole("tab", {name: "P&L"}).click();
    await expect(depthSlider).toHaveCount(1);
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

test("problems badge shows the deliberate problem count", async ({page}) => {
    await page.goto("/");

    // WP-09's three deliberate problems + WP-10's three stock warnings
    // (no future-dated txns at the pinned clock).
    await expect(page.getByRole("button", {name: "6 problems"})).toBeVisible();
});
