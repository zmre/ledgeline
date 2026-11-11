// E2E for Insights' startup metrics (plans/37) against the REAL stack: the
// ledgeline engine serving fixtures/saas/startup.journal on :5094
// (playwright.config.ts webServer) with the built SPA in front. Same pinned
// clock + localStorage seeding as e2e/insights.e2e.ts.
//
// At the pinned clock (2026-07-08) the default "Year over year" preset is the
// span 2024-07-01 … 2026-06-30: previous half 2024-07 … 2025-06, current half
// 2025-07 … 2026-06, so the current half's latest complete month is 2026-06.
//
// Figures, from fixtures/saas/README.md and checked against the live
// /api/saas response for that span:
//   - ARR $70,200 (MRR $5,850 + usage $100), net new ARR $27,000 (CMGR 4.1%)
//   - customers 8 (1 new, 3 churned, growth 0%)
//   - NRR 120.83%, GRR 90.28%; gross margin 81.86% (prev 80%)
//   - burn multiple 1.63× ("Good") on burn adjusted for prepayments, $44,070
//     (A3; raw cash net burn is $43,720)
//   - runway 117.01 months, cash-out 2036-03-31, on burn adjusted for
//     prepayments ($3,693.33/mo, 3-month average); cash burn $6,543.33/mo
//     (Amendment A1.1: asOf 2026-07-08)
//   - bridge: start $3,600, new $1,500 (1), expansion $1,100, contraction
//     $150, churn $800 (3: Fjord Studio, Lumen Labs, Pinecrest Dental),
//     reactivation $600 (2), end $5,850
//   - segments at 2026-06: mid $2,900, enterprise $2,200, smb $750
//
// Read-only: the engine serves the committed fixture in place, and nothing
// here saves.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5094";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            // Seeded only when absent, so a reload keeps what the test changed
            // (the "More metrics" open state lives in the same settings blob).
            if (localStorage.getItem("ledgeline.settings.v1") === null) {
                localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
            }
        },
        [API_URL, API_TOKEN]
    );
});

test("startup metrics: the headline boxes show the fixture's figures", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("startup-metrics")).toBeVisible();

    await expect(page.getByTestId("insights-box-arr")).toContainText("ARR (2026-06 × 12)");
    await expect(page.getByTestId("insights-box-arr-big")).toHaveText("$70,200.00");
    await expect(page.getByTestId("insights-box-arr")).toContainText("MRR $5,850.00 · usage $100.00");
    await expect(page.getByTestId("insights-box-arr-small")).toContainText("$27,000.00 (62.5%)");

    await expect(page.getByTestId("insights-box-netnewarr-big")).toHaveText("$27,000.00");
    await expect(page.getByTestId("insights-box-netnewarr-small")).toHaveText("CMGR 4.1%/mo");

    await expect(page.getByTestId("insights-box-customers-big")).toHaveText("8");
    await expect(page.getByTestId("insights-box-customers")).toContainText("1 new · 3 churned");
    await expect(page.getByTestId("insights-box-customers-small")).toContainText("0.0% growth");

    await expect(page.getByTestId("insights-box-nrr-big")).toHaveText("120.8%");
    await expect(page.getByTestId("insights-box-nrr")).toContainText("trailing 12 months");
    await expect(page.getByTestId("insights-box-nrr-small")).toHaveText("GRR 90.3%");

    await expect(page.getByTestId("insights-box-burnmultiple-big")).toHaveText("1.6×");
    await expect(page.getByTestId("insights-box-burnmultiple")).toContainText("Good");
    await expect(page.getByTestId("insights-box-burnmultiple-small")).toHaveText("net burn $44,070.00 (adjusted for prepayments)");
    // The YoY span ends at the last month with revenue booked: no horizon note.
    await expect(page.getByTestId("startup-closed-through")).toHaveCount(0);

    await expect(page.getByTestId("insights-box-grossmargin-big")).toHaveText("81.9%");
    await expect(page.getByTestId("insights-box-grossmargin-small")).toContainText("1.9 pp");

    // Every customer is tagged: no hidden-customers note, no inferred footnote.
    await expect(page.getByTestId("startup-customers-hidden")).toHaveCount(0);
    await expect(page.getByTestId("startup-inferred-note")).toHaveCount(0);
});

test("startup metrics: a business swaps Net Worth for Runway and hides boxes that don't apply", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("insights-box-runway-big")).toHaveText("117 months");
    await expect(page.getByTestId("insights-box-runway-small")).toHaveText(
        "cash-out ~2036-03 · burn $3,693.33/mo adjusted for prepayments · $6,543.33/mo cash (3-mo avg)"
    );
    await expect(page.getByTestId("insights-box-networth")).toHaveCount(0);
    // No holdings and no stocks in a SaaS company's books.
    await expect(page.getByTestId("insights-box-investment")).toHaveCount(0);
    await expect(page.getByTestId("insights-box-movers")).toHaveCount(0);
    // The rest are still there.
    await expect(page.getByTestId("insights-box-cash-big")).toHaveText("$432,160.00");
    await expect(page.getByTestId("insights-box-toptxns")).toBeVisible();
});

test("startup metrics: the segment table opens with three segments", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("startup-segments")).toHaveCount(0);
    await page.getByTestId("insights-box-arr-segments-toggle").click();

    const rows = page.getByTestId("startup-segments-row");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("mid");
    await expect(rows.nth(0)).toContainText("$2,900.00");
    await expect(rows.nth(1)).toContainText("enterprise");
    await expect(rows.nth(1)).toContainText("$2,200.00");
    await expect(rows.nth(2)).toContainText("smb");
    await expect(rows.nth(2)).toContainText("$750.00");

    // segment, product and class are offered, so the picker is there.
    await page.getByTestId("startup-segments-key").selectOption("product");
    await expect(page.getByTestId("startup-segments-row")).toHaveCount(2);
});

test("startup metrics: the bridge's churn row opens to its customers, and each links to the journal", async ({page}) => {
    await page.goto("/reports");
    const bridge = page.getByTestId("startup-bridge");
    await expect(page.getByTestId("startup-bridge-start")).toContainText("$3,600.00");
    await expect(page.getByTestId("startup-bridge-end")).toContainText("$5,850.00");
    await expect(page.getByTestId("startup-bridge-churn")).toContainText("(3)");
    await expect(page.getByTestId("startup-bridge-churn")).toContainText("$800.00");

    // The row's toggle, not its (i) help button: only the toggle has aria-expanded.
    await page.getByTestId("startup-bridge-churn").getByRole("button", {expanded: false}).click();
    const customers = page.getByTestId("startup-bridge-churn-customers").getByRole("link");
    await expect(customers).toHaveCount(3);
    await expect(customers.nth(0)).toContainText("Fjord Studio");
    await expect(customers.nth(1)).toContainText("Lumen Labs");
    await expect(customers.nth(2)).toContainText("Pinecrest Dental");
    await expect(bridge).toBeVisible();

    await customers.nth(0).click();
    // Lands on the journal, searching that customer across all dates.
    await expect(page).toHaveURL(/[?&]q=Fjord\+Studio/);
    await expect(page).toHaveURL(/[?&]preset=all/);
    await expect(page.getByLabel("Search transactions")).toHaveValue("Fjord Studio");
    // Fjord's 18 invoices, 17 collected payments (June's is outstanding) and
    // the August 2025 service credit.
    const footer = page.locator("footer");
    await expect(footer).toContainText("all dates");
    await expect(footer).toContainText("36 transactions");
    await expect(page.locator("tbody tr").first()).toContainText("Fjord Studio");
});

test("startup metrics: More metrics opens, and stays open across a reload", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("startup-more")).toBeVisible();
    await expect(page.getByTestId("startup-more-arpa")).toHaveCount(0);

    await page.getByRole("checkbox", {name: "Toggle More metrics"}).check();
    await expect(page.getByTestId("startup-more-arpa")).toHaveText("$731.25");
    await expect(page.getByTestId("startup-more-acv")).toHaveText("$18,000.00");
    await expect(page.getByTestId("startup-more-logo-churn")).toHaveText("37.5%");
    await expect(page.getByTestId("startup-more-deferred-revenue")).toHaveText("$10,400.00");
    await expect(page.getByTestId("startup-more-billings")).toHaveText("$68,950.00");

    await page.reload();
    await expect(page.getByTestId("startup-more-arpa")).toHaveText("$731.25");
});

test("startup metrics: the export downloads a workbook with five sheets", async ({page}) => {
    await page.goto("/reports");
    await expect(page.getByTestId("startup-metrics")).toBeVisible();

    const button = page.getByRole("button", {name: "Export startup metrics as xlsx"});
    await expect(button).toHaveText("Export .xlsx");
    const [download] = await Promise.all([page.waitForEvent("download"), button.click()]);
    expect(download.suggestedFilename()).toBe("startup-metrics-2024-07-01-to-2026-06-30.xlsx");

    // exceljs is CommonJS: under Playwright's ESM loader its classes sit on `default`.
    const {default: ExcelJS} = await import("exceljs");
    const workbook = new ExcelJS.Workbook();
    await workbook.xlsx.readFile(await download.path());
    expect(workbook.worksheets.map((ws) => ws.name)).toEqual(["Metrics", "Periods", "By segment", "Customer MRR", "Definitions"]);
    // The current half's ARR, as a real number in the base currency.
    const periods = workbook.getWorksheet("Periods");
    const arrRow = periods?.getColumn(1).values.indexOf("ARR") ?? -1;
    expect(periods?.getCell(arrRow, 2).value).toBe(70200);
});

// A2: a custom span running past the last month with revenue booked. The
// metrics stop at 2026-06 (the fixture's last recognition month) rather than
// reading July as every customer churning, and the heading says so.
test("startup metrics: a span past the books' last closed month says Closed through", async ({page}) => {
    await page.goto("/reports?tab=insights&istart=2024-11-01&iend=2026-10-31");
    await expect(page.getByTestId("startup-closed-through")).toContainText("Closed through 2026-06");
    await expect(page.getByTestId("insights-box-arr-big")).toHaveText("$70,200.00");
});
