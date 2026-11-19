// Revenue and expense recognition, end to end (plan 45 Phases 3–4), against
// the accrual engine on :5092 — a scratch copy of fixtures/accrual/ staged by
// playwright.config.ts (target/e2e-accrual/), because these tests WRITE:
// recognition entries, a schedule, a settlement link.
//
// Serial: every test runs against the same copy, in order — the reads first,
// then Post through Nov 17, then the journal's grouping of what it posted,
// then a new schedule from the editor, then an invoice link with a fee.
//
// Fixture facts at TODAY = 2026-11-17 (fixtures/accrual/README.md, and the
// endpoint tests in crates/ledgeline-server/tests/accrual_endpoints.rs):
//   - 10 entries due (the November occurrences), 1 due in a closed period
//     (Alpine Ski House 2026-09-10; closed-through 2026-09-30);
//   - 1 deferral needs a schedule: 2026-11-02 Fourth Coffee Invoice 2004;
//   - 3 invoice proposals: Tailspin Invoice 2001 ↔ its 2026-07-20 payment, $30
//     short (a fee, to check); Wingtip 2002, exact and clear; and the Gusto
//     payroll bill (2026-11-01), paid by the 2026-10-29 debit three days
//     before it, exact and clear;
//   - so the badge is 10 + 1 + 2 = 13;
//   - assets:prepaid ties, and liabilities:deferred-revenue is off by $100.00
//     on purpose (four fixture cases add up to it: see the README's "The Due
//     table's ties").
//
// Written in Phase 3 and NOT run by the agent that wrote it (agents never run
// Playwright); the lead runs it in Phase 4. Two scenarios the plan lists are
// `test.fixme` below, with what they need.

import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5092";
const SAMPLE_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const COPY = resolve(import.meta.dirname, "../../target/e2e-accrual");

async function connect(page: Page, url: string): Promise<void> {
    await page.addInitScript(
        ([server, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: server, serverToken: token, insightsOpen: false}));
        },
        [url, API_TOKEN]
    );
}

/** The span whose `data-tip` is the badge's breakdown. */
const badgeTip = (page: Page) => page.getByTestId("recognition-toolbar").locator(".tooltip").first();

const read = (file: string): string => readFileSync(resolve(COPY, file), "utf8").replaceAll("\r\n", "\n");
/** How many times `needle` occurs in `text`. */
const count = (text: string, needle: string): number => text.split(needle).length - 1;

test.describe.configure({mode: "serial"});

test.describe("accrual books (:5092)", () => {
    test.beforeEach(async ({page}) => {
        await page.clock.setFixedTime(FIXED_NOW);
        await connect(page, API_URL);
    });

    test("the badge shows the due count and the Due table ties", async ({page}) => {
        await page.goto("/");
        await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
        // The badge counts everything to act on; its tooltip breaks it down.
        await expect(page.getByTestId("recognition-count")).toHaveText("13");
        await expect(badgeTip(page)).toHaveAttribute("data-tip", /^Recognition: 10 due · 1 needs a schedule · 2 ready to link · 1 due in closed periods/);

        await page.keyboard.press("a");
        const overlay = page.getByTestId("recognition-overlay");
        await expect(overlay).toBeVisible();
        const november = overlay.getByTestId("due-month").filter({hasText: "Nov 2026"});
        await expect(november).toContainText("10");
        // Prepaid ties at the month's end. Deferred revenue is off by exactly
        // $100.00, on purpose: Fourth Coffee's unscheduled −$1,800, Adatum's
        // +$1,000 credit, the unposted closed September −$100 and the duplicate
        // +$1,000 (fixtures/accrual/README.md, "The Due table's ties").
        const ties = overlay.getByTestId("due-tie");
        await expect(ties.filter({hasText: "assets:prepaid"}).first()).toContainText("ties ✓");
        const deferred = ties.filter({hasText: "liabilities:deferred-revenue"}).first();
        await expect(deferred).toContainText("$-23,616.16 → $-18,882.83");
        await expect(deferred).toContainText("off by $100.00");
        // The closed period's entry is listed apart, unchecked.
        await expect(overlay.getByTestId("due-closed")).toContainText("1");
        await expect(overlay.getByTestId("due-closed-check")).not.toBeChecked();
        // Needs a schedule lists the external invoice.
        await overlay.getByTestId("recognition-tab-needs").click();
        await expect(overlay.getByTestId("needs-item")).toHaveCount(1);
        await expect(overlay.getByTestId("needs-item")).toContainText("Fourth Coffee Invoice 2004");
    });

    test("Post through Nov 17 writes the entries once; the badge goes to 0", async ({page}) => {
        const before = read("books.journal");
        expect(count(before, "recognized: 2026-11-")).toBe(0);
        await page.goto("/");
        await expect(page.getByTestId("recognition-count")).toHaveText("13");
        await page.keyboard.press("a");
        const overlay = page.getByTestId("recognition-overlay");
        const post = overlay.getByTestId("recognition-post");
        await expect(post).toContainText("Nov 17");
        await post.click();
        await expect(overlay.getByTestId("recognition-notice")).toContainText("Posted 10 recognition entries through 2026-11-17");
        // Nothing due; the schedule-less invoice and the two clear matches remain.
        await expect(page.getByTestId("recognition-count")).toHaveText("3");
        await expect(badgeTip(page)).toHaveAttribute("data-tip", /^Recognition: 0 due · 1 needs a schedule · 2 ready to link/);

        const after = read("books.journal");
        expect(count(after, "recognized: 2026-11-")).toBe(10);
        // Nothing open is left: no month rows, only the closed period's entry,
        // still listed apart and unchecked. A reload changes no bytes.
        await expect(overlay.getByTestId("due-month")).toHaveCount(0);
        await expect(overlay.getByTestId("due-closed")).toContainText("1");
        await expect(overlay.getByTestId("due-closed-check")).not.toBeChecked();
        await page.keyboard.press("Escape");
        await page.reload();
        await expect(page.getByTestId("recognition-count")).toHaveText("3");
        expect(read("books.journal")).toBe(after);
    });

    test("the journal groups November's entries, expands and hides them, and the totals don't move", async ({page}) => {
        await page.goto("/?from=2026-11-01&to=2026-11-30");
        const group = page.getByTestId("recognition-group-row").first();
        await expect(group).toContainText("Recognition · Nov 2026 · 10 entries");

        // Expand in place: the ten members show right under the group row.
        await page.getByTestId("recognition-toggle").first().click();
        await expect(page.locator("tbody tr[data-txn]").filter({hasText: "Recognized"})).toHaveCount(10);
        await page.getByTestId("recognition-toggle").first().click();

        // Hidden: no rows, the footer says so, and the totals are the same.
        const chip = page.getByTestId("recognition-chip");
        await expect(chip).toHaveText(/Grouped/);
        await chip.click(); // Grouped → Hidden
        await expect(chip).toHaveText(/Hidden/);
        await expect(page.getByTestId("recognition-group-row")).toHaveCount(0);
        await expect(page.getByTestId("footer-recognition-hidden")).toContainText("incl. 10 hidden recognition entries");
        // The figures are the engine's, so compare them there: hiding takes rows
        // away, never postings (D8, Q11).
        const summary = async (mode: string): Promise<unknown> => {
            const response = await page.request.get(`${API_URL}/api/journal/summary?from=2026-11-01&to=2026-11-30&recognition=${mode}`, {
                headers: {Authorization: `Bearer ${API_TOKEN}`},
            });
            const body = (await response.json()) as {visibleNet: unknown; perCommodity: unknown};
            return {visibleNet: body.visibleNet, perCommodity: body.perCommodity};
        };
        expect(await summary("hide")).toEqual(await summary("group"));
        expect(await summary("show")).toEqual(await summary("group"));
        await chip.click(); // Hidden → Shown
        await chip.click(); // Shown → Grouped (the remembered preference goes back)
        await expect(chip).toHaveText(/Grouped/);
    });

    test("a new schedule from the editor: the exclusive `to` rule in the preview, and the bytes in recognition.journal", async ({page}) => {
        const rulesBefore = read("recognition.journal");
        await page.goto("/");
        // `a` is bound once the accrual status has loaded: wait for the badge.
        await expect(page.getByTestId("recognition-count")).toBeVisible();
        await page.keyboard.press("a");
        const overlay = page.getByTestId("recognition-overlay");
        await overlay.getByTestId("recognition-tab-needs").click();
        await overlay.getByTestId("needs-edit").first().click();

        const section = page.getByTestId("accrual-section");
        await expect(section).toBeVisible();
        await section.getByTestId("accrual-recognize").first().click();
        const form = section.getByTestId("accrual-schedule-form");
        await form.getByTestId("accrual-term-12").click();
        await form.getByTestId("accrual-target").fill("revenue:subscriptions");
        // The live preview: twelve periods, and the rule's `to` is the day after
        // the last entry (hledger's exclusive end), as the file will hold it.
        const preview = section.getByTestId("accrual-schedule-preview");
        await expect(preview.getByTestId("accrual-period")).toHaveCount(12);
        await expect(preview.getByTestId("accrual-rule-text").first()).toContainText("~ monthly from 2026-11-02 to 2027-10-03");

        await page.getByTestId("review-primary").click();
        await expect(page.getByTestId("accrual-section")).toBeHidden();

        const rules = read("recognition.journal");
        expect(rules.startsWith(rulesBefore.trimEnd())).toBe(true);
        expect(rules).toContain("~ monthly from 2026-11-02 to 2027-10-03");
        // The source posting carries the same id the new rule group does.
        const id = /recognition: ([a-z0-9]{6})/.exec(rules.slice(rulesBefore.length))?.[1];
        expect(id).toBeTruthy();
        expect(read("books.journal")).toMatch(new RegExp(`liabilities:deferred-revenue[^\\n]*recognition: ${id}`));
    });

    test.fixme("the file-creation prompt on a ledger with no recognition.journal", async () => {
        // Needs a second scratch copy without recognition.journal: a webServer
        // in playwright.config.ts on its own port (5091) staging
        // target/e2e-accrual-nofile/ as the :5092 entry does, minus
        // recognition.journal. The agent that wrote this could not add that
        // entry (its edit was refused), so it waits. Then: open Fourth Coffee's
        // editor from Needs a schedule, Recognize over time, 12 months,
        // revenue:subscriptions, Save → `accrual-file-prompt` names
        // recognition.journal and says the books don't include it; Cancel
        // writes nothing (both files unchanged); Save again → Create and save
        // writes the rule group into a new recognition.journal and the
        // `recognition:` tag onto the deferred line.
    });

    test.fixme("renewal prefill and a restatement with a closed period", async () => {
        // Renewal: edit a new Customer X prepayment (saved first: add mode has no
        // prefill) → `accrual-renewal-banner` "Prefilled from Customer X · …",
        // contract "renewal" marked inferred.
        // Restatement: edit Alpine Ski House's source (as0710) amount → Edit
        // schedule → `accrual-restatement` keeps the closed rows, rewrites the
        // open ones, and shows one `accrual-adjustment` dated today
        // (restated-through: 2026-09-30); Apply all writes it.
    });

    test("Details on either side opens both transactions, the matched posting highlighted; a payroll bill paid early pairs", async ({page}) => {
        await page.goto("/settlements");
        await expect(page.getByTestId("settlement-card")).toHaveCount(3);

        // Payroll: the Gusto draw came three days before its bill, exact and clear.
        const gusto = page.getByTestId("settlement-card").filter({hasText: "Gusto"});
        await expect(gusto.getByTestId("settlement-gap")).toHaveText("Paid 3 days before the invoice");
        await expect(gusto.getByTestId("settlement-clear")).toBeVisible();
        await expect(gusto.getByTestId("settlement-fit")).toHaveText("Exact amount");

        // Details on the PAYMENT side opens both sides, each read in full, each
        // with its own matched posting highlighted.
        const tailspin = page.getByTestId("settlement-card").filter({hasText: "2001"});
        await tailspin.getByTestId("settlement-payment").getByTestId("details-toggle").click();
        await expect(tailspin.getByTestId("txn-details")).toHaveCount(2);
        const invoice = tailspin.getByTestId("settlement-invoice");
        const payment = tailspin.getByTestId("settlement-payment");
        await expect(invoice.getByTestId("detail-description")).toHaveText("Tailspin Invoice 2001");
        await expect(payment.getByTestId("detail-description")).toContainText("Tailspin payment");
        await expect(invoice.locator("[data-highlighted]")).toHaveCount(1);
        await expect(payment.locator("[data-highlighted]")).toHaveCount(1);
        await expect(invoice.locator("[data-highlighted]")).toContainText("assets:accounts receivable");
        await expect(payment.locator("[data-highlighted]")).toContainText("assets:accounts receivable");
        // The other cards stay closed; the invoice side's Details closes both.
        await expect(gusto.getByTestId("txn-details")).toHaveCount(0);
        await invoice.getByTestId("details-toggle").click();
        await expect(tailspin.getByTestId("txn-details")).toHaveCount(0);
    });

    test("an invoice paid $30 short links with a fee", async ({page}) => {
        await page.goto("/settlements");
        const card = page.getByTestId("settlement-card").filter({hasText: "2001"});
        await expect(card).toBeVisible();
        await expect(card.getByTestId("settlement-fit")).toContainText("30");
        const fee = card.getByTestId("settlement-fee");
        if ((await fee.inputValue()) === "") await fee.fill("expenses:payment fees");
        await card.getByTestId("settlement-link").click();
        await expect(page.getByTestId("settlement-card").filter({hasText: "2001"})).toHaveCount(0);

        const books = read("books.journal");
        expect(books).toMatch(/expenses:payment fees\s+\$30\.00/);
        expect(count(books, "match:")).toBeGreaterThanOrEqual(2);
    });

    test("Link all links exactly the two clear pairs in one write, and the badge drops", async ({page}) => {
        // Before: one entry due (the Fourth Coffee schedule's Nov 2 occurrence,
        // set up above), nothing needs a schedule, two clear pairs.
        await page.goto("/");
        await expect(page.getByTestId("recognition-count")).toHaveText("3");
        await expect(badgeTip(page)).toHaveAttribute("data-tip", /^Recognition: 1 due · 0 need a schedule · 2 ready to link/);
        const generation = async (): Promise<number> => {
            const response = await page.request.get(`${API_URL}/api/accrual?today=2026-11-17`, {headers: {Authorization: `Bearer ${API_TOKEN}`}});
            const body = (await response.json()) as {generation: string};
            return Number.parseInt(body.generation.split("-").at(-1) ?? "", 16);
        };
        const before = await generation();
        const matchesBefore = count(read("books.journal"), "match:");

        await page.goto("/settlements");
        const linkAll = page.getByTestId("settlements-link-all");
        await expect(linkAll).toContainText("Link all clear (2)");
        await linkAll.click(); // asks first
        await linkAll.click(); // Link them
        await expect(page.getByTestId("settlements-linked-all")).toHaveText("Linked 2 pairs.");
        await expect(page.getByTestId("settlement-card")).toHaveCount(0);

        // ONE write: one new generation, and both pairs' tags (four) landed together.
        expect((await generation()) - before).toBe(1);
        const books = read("books.journal");
        expect(count(books, "match:") - matchesBefore).toBe(4);
        expect(books).toMatch(/Gusto debit\n[^\n]*\n\s+liabilities:accounts payable\s+\$4812\.33\s+;[^\n]*match: ?[a-z0-9]+/);

        await page.goto("/");
        await expect(page.getByTestId("recognition-count")).toHaveText("1");
        await expect(badgeTip(page)).toHaveAttribute("data-tip", /^Recognition: 1 due · 0 need a schedule · 0 ready to link/);
    });
});

test.describe("household sample (:5099)", () => {
    test("shows no Recognition button, editor section, chip or Books tab", async ({page}) => {
        await connect(page, SAMPLE_URL);
        // Absence only means something once the gate has answered: wait for it.
        const gate = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/accrual" && response.ok());
        // The whole sample, so `j` has a row to land on whatever today is.
        await page.goto("/?from=2000-01-01&to=2099-12-31");
        expect(await (await gate).json()).toEqual({enabled: false});
        await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
        await expect(page.getByTestId("recognition-button")).toHaveCount(0);
        await expect(page.getByTestId("recognition-chip")).toHaveCount(0);
        await page.keyboard.press("a");
        await expect(page.getByTestId("recognition-overlay")).toHaveCount(0);
        await page.keyboard.press("j");
        await page.keyboard.press("Enter");
        await expect(page.getByTestId("review-overlay")).toBeVisible();
        await expect(page.getByTestId("accrual-section")).toHaveCount(0);
        await page.goto("/settings?tab=books");
        await expect(page.getByRole("tab", {name: "Books"})).toHaveCount(0);
    });
});
