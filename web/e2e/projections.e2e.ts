// E2E for the Projections tab against the REAL stack: the ledgeline engine
// serving fixtures/sample.journal on :5099 (playwright.config.ts webServer) with
// the built SPA in front. Same pinned clock + localStorage seeding as the other
// suites.
//
// # READ-ONLY, deliberately
//
// Nothing here writes a file, for the reason `budget.e2e.ts` gives at length:
// the e2e engine is launched over a COMMITTED fixture that five other specs
// assert exact numbers from, there is no scratch journal to redirect a save
// into, and a spec that wrote one would leave the working tree dirty and stop
// being idempotent the second time it ran.
//
// Phase 3 adds Save As, and the rule does not change: the write path is proved
// in `crates/ledgeline-core/tests/projection_files.rs` and
// `crates/ledgeline-server/tests/projection_endpoints.rs`, AGAINST THE BYTES,
// which is a far stronger check than clicking a button. What is asserted here
// is the half those cannot reach: that the file controls are on the page, that
// the picker lists what the engine found, and that the Save As dialog shows the
// path it would write BEFORE it writes it. The dialog is opened and cancelled;
// its Save button is never pressed.
//
// # Fixture facts
//
// `fixtures/sample.journal` declares no `~` periodic rules, so EVERY revenue and
// expense category in it is unbudgeted: the seed is entirely
// `source: "unbudgeted"` rows, each described as an average of its trailing
// window. That is the same fact `budget.e2e.ts` relies on for its gaps section.
//
// Note the one thing this spec cannot pin: the engine takes opening balances as
// of ITS today, and `page.clock` only moves the browser's. So nothing here
// asserts a projected figure — only that the tab is reachable, seeds, and
// renders all six readings (plan 28), the statements with their divider.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

test("projections: is a top-level tab, reachable from the nav", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Projections"}).click();
    await expect(page).toHaveTitle("Ledgeline — Projections");
    await expect(page.getByRole("heading", {name: "Projections", exact: true})).toBeVisible();
});

test("projections: seeds the what-if table from the journal, flagging what it estimated", async ({page}) => {
    await page.goto("/projections");

    // Both recurring sections exist, and the seed put rows in them.
    await expect(page.getByTestId("projection-section-income")).toBeVisible();
    await expect(page.getByTestId("projection-section-expense")).toBeVisible();
    await expect(page.getByTestId("projection-line").first()).toBeVisible();

    // sample.journal has no `~` rules, so every seeded row is an average of the
    // trailing twelve months — and says so, rather than reading as something the
    // user wrote.
    const note = page.getByLabel("Description for income:salary");
    await note.hover();
    await expect(page.getByTestId("note-tooltip")).toContainText("average over");

    // Income is a MAGNITUDE in the box, though the wire signs revenue negative.
    await expect(page.getByLabel("Amount for income:salary")).toHaveValue(/^\d/);
});

// UNVERIFIED: written against the markup, never executed — Playwright cannot
// run in the environment this was added in (plan 23, Phase 3).
test("projections: the Assets and balances section is there, empty, on a seeded journal", async ({page}) => {
    await page.goto("/projections");

    // `sample.journal` has no `~` rules at all, so the seed produces flows only
    // — `budget_gaps` measures revenue and expense accounts. The section is
    // still present, with its own empty state and its own Add button, because
    // an asset row is something the user creates deliberately.
    const assets = page.getByTestId("projection-section-asset");
    await expect(assets).toBeVisible();
    await expect(assets).toContainText("A brokerage account, a 401k, a house.");
    await expect(assets.getByRole("button", {name: "+ Add an asset"})).toBeEnabled();
    await expect(page.getByTestId("projection-asset-line")).toHaveCount(0);
});

// UNVERIFIED: written against the markup, never executed — Playwright cannot
// run beside the other agents this was added alongside (plan 30 Phase 4).
// Read-only like the rest of this file: a rule added in the table makes the
// scenario dirty and re-runs it; nothing is saved. No figure is asserted.
test("projections: an automatic posting rule is added and edited as a form", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();

    const section = page.getByTestId("projection-section-auto");
    await expect(section).toContainText("Nothing here yet");
    await section.getByRole("button", {name: "+ Add a rule"}).click();
    await expect(section.getByTestId("projection-auto-rule")).toHaveCount(1);
    await expect(section.getByText("forecasts only")).toBeVisible();

    await page.getByLabel("Matched account for rule 1").fill("income:salary");
    await page.getByLabel("Matched account for rule 1").press("Enter");
    await page.getByLabel("Account for posting 1 of rule 1").fill("expenses:taxes");
    await page.getByLabel("Account for posting 1 of rule 1").press("Enter");
    await page.getByLabel("Percent for posting 1 of rule 1").fill("10");
    await page.getByLabel("Percent for posting 1 of rule 1").blur();

    // The preview reads the seeded salary line; the projection re-runs with the
    // rule and still renders.
    await expect(section.getByTestId("auto-rule-preview")).toContainText("to expenses:taxes on each");
    await expect(page.getByTestId("projection-runway")).toBeVisible();

    await section.getByLabel("Row menu for rule 1").click();
    await page.getByTestId("row-menu").getByRole("menuitem", {name: "Delete rule"}).click();
    await expect(section.getByTestId("projection-auto-rule")).toHaveCount(0);
});

// UNVERIFIED, as above.
test("projections: adding an asset row shows the ledger's own balance, greyed", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();

    await page.getByRole("button", {name: "+ Add an asset"}).click();
    const row = page.getByTestId("projection-asset-line").first();
    await expect(row).toBeVisible();

    // Seeded into the journal's own asset tree, and a balance box that is EMPTY
    // — the journal's figure is its placeholder, which is what "greyed until
    // you type over it" means.
    await expect(row.getByRole("combobox").first()).toHaveValue("assets:");
    const balance = page.getByLabel(/^Balance for /);
    await expect(balance).toHaveValue("");

    // Naming a real account and letting the debounced recompute land puts the
    // ledger's figure in the placeholder.
    await row.getByRole("combobox").first().fill("assets:broker:taxable");
    await row.getByRole("combobox").first().press("Enter");
    await expect(page.getByLabel("Balance for assets:broker:taxable")).toHaveAttribute("placeholder", /^\d/);

    // Typing over it is visibly an override, and the ledger's figure survives.
    await page.getByLabel("Balance for assets:broker:taxable").fill("640000");
    await page.getByLabel("Balance for assets:broker:taxable").blur();
    await expect(page.getByTestId("ledger-balance")).toContainText("ledger");
    await page.getByLabel("Use the journal's balance for assets:broker:taxable").click();
    await expect(page.getByTestId("ledger-balance")).toHaveCount(0);
});

test("projections: opens on the runway over two years with two periods of actuals, and says so in the URL", async ({page}) => {
    await page.goto("/projections");

    await expect(page.getByRole("tab", {name: "Runway"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByLabel("Number of periods")).toHaveValue("24");
    await expect(page.getByLabel("Actual periods before the projection")).toHaveValue("2");
    await expect(page.getByLabel("Interval")).toHaveValue("monthly");
    await expect(page).toHaveURL(/tab=cash&interval=monthly&count=24&lookback=2/);
});

test("projections: the runway and statements render, the runway in words and the statements with their divider", async ({page}) => {
    await page.goto("/projections");

    await expect(page.getByTestId("projection-cash")).toBeVisible();
    // The question the tab exists for, answered above the picture of it.
    // sample.journal's seed builds cash: TOTAL cash never closes below zero,
    // so there is no runway (plan 28 amendment 22 — this used to be a false
    // "Cash turns negative in Jul 2026" off unvalued EUR seed lines).
    await expect(page.getByTestId("projection-runway")).toContainText("Cash never turns negative");
    await expect(page.getByTestId("projection-cash-chart")).toBeVisible();

    // The Reports tab's own statements, over actuals and then the forecast.
    await page.getByRole("tab", {name: "Balance sheet"}).click();
    const sheet = page.getByTestId("projection-bs");
    await expect(sheet.getByTestId("balance-sheet")).toBeVisible();
    await expect(sheet.getByTestId("bs-section-assets").getByTestId("projected-label")).toHaveText("Projected");
    await expect(sheet.getByTestId("bs-tie-out")).toContainText("Balanced");

    await page.getByRole("tab", {name: "Income statement"}).click();
    await expect(page.getByTestId("projection-is").getByTestId("income-statement")).toBeVisible();
    await expect(page.getByTestId("projection-is").getByTestId("projected-label").first()).toHaveText("Projected");

    await page.getByRole("tab", {name: "Cash flow"}).click();
    await expect(page.getByTestId("projection-cf").getByTestId("cash-flow-statement")).toBeVisible();
    await expect(page.getByTestId("projection-cf").getByTestId("projected-label").first()).toHaveText("Projected");
});

test("projections: picking a reading brings its tab strip to the top of the window, just under the pinned header", async ({page}) => {
    await page.setViewportSize({width: 1280, height: 800});
    await page.goto("/projections?tab=bs");
    await expect(page.getByTestId("projection-bs").getByTestId("balance-sheet")).toBeVisible();
    const strip = page.getByRole("tablist", {name: "Projection report"});
    const header = page.getByTestId("app-header");
    // From the bottom of a long balance sheet to Journal, whose panel is a
    // one-line placeholder until its run lands: the page must not shrink out
    // from under the strip and leave it at the bottom of the viewport.
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    for (const name of ["Journal", "Income statement", "Runway"]) {
        await page.getByRole("tab", {name}).click();
        await expect(page.getByRole("tab", {name})).toHaveAttribute("aria-selected", "true");
        await expect
            .poll(
                async () => {
                    const [s, h] = [await strip.boundingBox(), await header.boundingBox()];
                    return s === null || h === null ? Infinity : Math.abs(Math.round(s.y - (h.y + h.height)));
                },
                {message: `${name}: strip just under the header`}
            )
            .toBeLessThanOrEqual(1);
    }
});

test("the header stays pinned at the top of the window while the page scrolls", async ({page}) => {
    await page.setViewportSize({width: 1280, height: 600});
    await page.goto("/projections?tab=bs");
    await expect(page.getByTestId("projection-bs").getByTestId("balance-sheet")).toBeVisible();
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(200);
    const header = page.getByTestId("app-header");
    await expect(header).toBeInViewport();
    expect(Math.round((await header.boundingBox())?.y ?? -1)).toBe(0);
    await expect(header.getByRole("link", {name: "Projections"})).toBeVisible();
});

test("projections: the Accounts tab reads assets as balances with a Δ and flows as changes, with empty accounts on request", async ({page}) => {
    await page.goto("/projections?tab=accounts");
    await expect(page.getByRole("tab", {name: "Accounts"})).toHaveAttribute("aria-selected", "true");
    const view = page.getByTestId("projection-accounts");
    const table = view.getByTestId("accounts-table");
    await expect(table).toBeVisible();
    await expect(table.getByTestId("projected-label")).toHaveText("Projected");
    await expect(page).toHaveURL(/empty=0/);
    // Each section says how it reads, and its last column follows.
    const assets = table.locator('[data-testid="account-section"][data-section="assets"]');
    await expect(assets).toHaveAttribute("data-reading", "balance");
    await expect(assets.locator('[data-column="summary-heading"]')).toHaveText("Δ over the period");
    await expect(assets.locator('[data-testid="account-section-total"] [data-column="summary"]')).toContainText("Δ");
    const expenses = table.locator('[data-testid="account-section"][data-section="expenses"]');
    await expect(expenses).toHaveAttribute("data-reading", "change");
    await expect(expenses.locator('[data-column="summary-heading"]')).toHaveText("Total");
    // The help text says it in words.
    await expect(view.getByTestId("account-report-help")).toContainText("balance at the end of each period");
    // No Change/Balance switch any more.
    await expect(view.getByRole("button", {name: "Balance"})).toHaveCount(0);
    // `expenses:shopping` is declared in sample.journal and never posted to.
    await expect(table.locator('[data-account="expenses:shopping"]')).toHaveCount(0);
    await view.getByRole("checkbox", {name: "Show empty accounts"}).check();
    await expect(page).toHaveURL(/empty=1/);
    await expect(table.locator('[data-account="expenses:shopping"]')).toHaveCount(1);
});

test("projections: the Journal tab lists the generated transactions only, each naming its rule", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    // Digit 6 is the Journal tab (focus is on the page, not in a field).
    await page.locator("h1").click();
    await page.keyboard.press("6");
    await expect(page.getByRole("tab", {name: "Journal"})).toHaveAttribute("aria-selected", "true");
    await expect(page).toHaveURL(/tab=journal/);
    const journal = page.getByTestId("forecast-journal");
    await expect(journal).toBeVisible();
    const first = journal.getByTestId("forecast-entry").first();
    await expect(first).toHaveAttribute("data-origin", "rule");
    // The seed is monthly rules, marked as hledger marks them.
    await expect(first).toContainText("~ monthly");
    await expect(journal.getByTestId("forecast-journal-count")).toContainText("Generated transactions 1–");

    // The search box narrows by the engine's own search text: an account the
    // first entry posts to finds it, and nonsense finds nothing.
    const account = (await first.locator("td[title]").first().getAttribute("title")) ?? "";
    expect(account).not.toBe("");
    const search = journal.getByTestId("forecast-journal-search");
    await search.fill(account.toUpperCase());
    await expect(journal.getByTestId("forecast-journal-count")).toContainText("Matching 1–");
    // Every entry still listed posts to an account containing the query.
    const accounts = await journal
        .getByTestId("forecast-entry")
        .evaluateAll((entries) => entries.map((entry) => [...entry.querySelectorAll("td[title]")].map((cell) => cell.getAttribute("title") ?? "")));
    expect(accounts.length).toBeGreaterThan(0);
    for (const names of accounts) expect(names.some((name) => name.includes(account))).toBe(true);
    await search.fill("zzz-no-such-text");
    await expect(journal.getByTestId("forecast-entry")).toHaveCount(0);
    await expect(journal.getByTestId("forecast-journal-count")).toContainText("No generated transaction matches");
    await search.fill("");
    await expect(journal.getByTestId("forecast-journal-count")).toContainText("Generated transactions 1–");
    await page.locator("h1").click();
    await page.keyboard.press("5");
    await expect(page.getByRole("tab", {name: "Accounts"})).toHaveAttribute("aria-selected", "true");
});

test("projections: a seeded row shows its inferred counter-account greyed", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    // Empty box, the engine's default as the placeholder — "greyed until you
    // type over it", as the Balance column is.
    const counter = page.getByLabel("Counter-account for expenses:housing");
    await expect(counter).toHaveValue("");
    await expect(counter).toHaveAttribute("placeholder", /^assets:/);
    // Inferred from the most common pairing in history, ANY account type:
    // depreciation books against its contra-asset, never cash.
    await expect(page.getByLabel("Counter-account for expenses:depreciation")).toHaveAttribute("placeholder", "assets:vehicles:car:depreciation");
    await expect(page.getByLabel("Default cash account")).toHaveAttribute("placeholder", /^assets:/);
});

test("projections: a bookmarked URL reopens the same tab and window", async ({page}) => {
    await page.goto("/projections?tab=is&interval=yearly&count=5&lookback=1&depth=1");

    await expect(page.getByRole("tab", {name: "Income statement"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByLabel("Interval")).toHaveValue("yearly");
    await expect(page.getByLabel("Number of periods")).toHaveValue("5");
    await expect(page.getByLabel("Actual periods before the projection")).toHaveValue("1");
});

test("projections: a link to a reading plan 28 retired opens the runway instead", async ({page}) => {
    await page.goto("/projections?tab=worth&interval=monthly&count=6&depth=2");
    await expect(page.getByRole("tab", {name: "Runway"})).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
});

test("projections: editing the table marks it edited and recomputes", async ({page}) => {
    // The whole point of carrying the scenario in the request body: an UNSAVED
    // edit projects. Asserted as a request going out and the page saying the
    // table has been edited — never as a figure, which depends on the engine's
    // own today.
    const runs: string[] = [];
    page.on("request", (request) => {
        if (request.url().includes("/api/projections/run")) runs.push(request.url());
    });

    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    const before = runs.length;

    await page.getByLabel("Amount for expenses:housing").fill("2500");
    await page.getByLabel("Amount for expenses:housing").blur();

    await expect(page.getByTestId("projection-dirty")).toBeVisible();
    await expect.poll(() => runs.length).toBeGreaterThan(before);
    // Nothing was written — an edit is an edit, and the "edited" badge stays up
    // until a save clears it. This spec never saves.
    await expect(page.getByTestId("projection-file-select")).toHaveValue("");
});

test("projections: a reload with a scenario file open shows its table, not a spinner", async ({page}) => {
    // The remembered file (decision 10) is reopened on mount INSTEAD of
    // seeding, and the table used to be gated on the seed alone — so with a
    // file open every reload span over the table forever, with no request
    // pending and nothing wrong in the engine. A read, never a write: this
    // fixture belongs to the parity suite.
    const id = "projections-parity/flows.journal";
    await page.addInitScript(
        ([url, token, remembered]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token, lastProjectionId: remembered}));
        },
        [API_URL, API_TOKEN, id]
    );

    await page.goto("/projections");
    for (const visit of ["first load", "reload"]) {
        if (visit === "reload") await page.reload();
        await expect(page.getByTestId("projection-file-select"), visit).toHaveValue(id);
        await expect(page.getByTestId("projection-line").first(), visit).toBeVisible();
        await expect(page.getByLabel("Loading a starting scenario"), visit).toHaveCount(0);
        await expect(page.getByTestId("projection-cash"), visit).toBeVisible();
    }
});

test("projections: changing the window re-asks the engine and mirrors to the URL", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();

    await page.getByLabel("Interval").selectOption("quarterly");
    await expect(page).toHaveURL(/interval=quarterly/);
    await expect(page.getByTestId("projection-cash-chart")).toBeVisible();

    await page.getByLabel("Actual periods before the projection").fill("0");
    await page.getByLabel("Actual periods before the projection").blur();
    await expect(page).toHaveURL(/lookback=0/);
});

// ---------------------------------------------------------------------------
// Scenario files (plan 22, Phase 3) — READ-ONLY
//
// Nothing below writes. The picker is read, the dialog is opened and cancelled,
// and the one assertion that matters — the path shown before the write — is
// made against the dialog's own preview rather than against a file on disk.
// ---------------------------------------------------------------------------

test("projections: the file bar lists the journals beside the fixture", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-file-bar")).toBeVisible();

    const picker = page.getByTestId("projection-file-select");
    // A seeded scenario has no file of its own, and the picker says so rather
    // than showing a blank box.
    await expect(picker).toHaveValue("");
    await expect(picker.locator("option", {hasText: "Seeded from your journal"})).toHaveCount(1);
    // The fixture journal itself is a `*.journal` beside itself, so it is
    // listed — "really any journal file with budget-like entries could be used".
    await expect(picker.locator("optgroup")).toHaveCount(1);
    await expect(picker.locator('optgroup[label="Other journals"]')).toHaveCount(1);

    // Save is offered but not possible: there is no file to save OVER yet.
    await expect(page.getByTestId("projection-save")).toBeDisabled();
    await expect(page.getByTestId("projection-save-as")).toBeEnabled();
});

test("projections: the fixture journal is loadable and marked read-only", async ({page}) => {
    // DECISION 5: a file the main journal is parsed from can be READ (the ask
    // wants the active budget file as a default) and can never be saved over.
    await page.goto("/projections");
    await expect(page.getByTestId("projection-file-bar")).toBeVisible();

    await page.getByTestId("projection-file-select").selectOption({label: "sample"});
    await expect(page.getByTestId("projection-read-only")).toContainText("part of your main journal");
    await expect(page.getByTestId("projection-save")).toBeDisabled();
    // The table is still there, still projecting, just from a different source.
    await expect(page.getByTestId("projection-cash")).toBeVisible();
});

test("projections: Save as shows the path it would write, and cancels without writing", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-file-bar")).toBeVisible();

    await page.getByTestId("projection-save-as").click();
    await expect(page.getByTestId("projection-save-dialog")).toBeVisible();

    // A name with no letters or digits has no file name, and the button says so
    // rather than the server doing it.
    await page.getByTestId("projection-save-name").fill("!!!");
    await expect(page.getByTestId("projection-save-blocker")).toContainText("no letters or digits");
    await expect(page.getByTestId("projection-save-submit")).toBeDisabled();

    // …and a real one previews the exact path, before anything is written.
    await page.getByTestId("projection-save-name").fill("Series A with a hiring ramp");
    await expect(page.getByTestId("projection-save-path")).toContainText("projection-series-a-with-a-hiring-ramp.journal");
    await expect(page.getByTestId("projection-save-submit")).toBeEnabled();

    // Cancelled. NOTHING is written by this suite.
    await page.getByTestId("projection-save-cancel").click();
    await expect(page.getByTestId("projection-save-dialog")).toHaveCount(0);
});

test("projections: every reading downloads as an xlsx workbook", async ({page}) => {
    await page.goto("/projections");
    await expect(page.getByTestId("projection-cash")).toBeVisible();
    for (const [name, slug] of [
        ["Runway", "runway"],
        ["Balance sheet", "balance-sheet"],
        ["Income statement", "income-statement"],
        ["Cash flow", "cash-flow"],
        ["Accounts", "accounts"],
        ["Journal", "journal"],
    ] as const) {
        await page.getByRole("tab", {name}).click();
        const button = page.getByRole("button", {name: `Export ${name} as xlsx`});
        // Accounts and Journal appear once their run lands.
        await expect(button).toBeVisible();
        const [download] = await Promise.all([page.waitForEvent("download"), button.click()]);
        expect(download.suggestedFilename()).toMatch(new RegExp(`^projection-scenario-${slug}-\\d{4}-\\d{2}-\\d{2}-to-\\d{4}-\\d{2}-\\d{2}\\.xlsx$`));
        const path = await download.path();
        const {size} = await import("node:fs").then((fs) => fs.promises.stat(path));
        // An xlsx is a zip: never empty, and starts "PK".
        expect(size).toBeGreaterThan(1000);
        const head = await import("node:fs").then((fs) => fs.promises.readFile(path).then((bytes) => bytes.subarray(0, 2).toString()));
        expect(head).toBe("PK");
    }
});
