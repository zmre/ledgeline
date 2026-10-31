// Transaction review, end to end (plan 32 Phase 3), against the review engine
// on :5098 — a scratch copy of fixtures/review/ (see playwright.config.ts),
// so the saves here never touch a committed fixture or sample.journal's totals.
//
// What jsdom cannot show and this does: the real keymap and focus model, ⌘⇧↵
// reaching the page unswallowed (D16), Tab completing in a real combobox, and
// the BYTES the batch wrote, read straight from the copy on disk.
//
// Fixture facts at TODAY = 2026-10-30 with the default 365-day window, read
// from the live engine (`GET /api/review/queue`):
//   Unknowns, newest first (8):
//     #25 Venmo payment 8812 (reviewed:2026-10-21 but still unknown; no suggestion)
//     #23 NFLX DIGITAL 8812 (recurring → expenses:streaming, 60%, strong)
//     #22 SQ *BLUE BOTTLE 4412 OAKLAND (exact key → expenses:food:coffee 83%,
//         strong, with the rename "Blue Bottle Coffee"); similar: 7 candidates
//         (#27 future, #20, #19, #14 coffee, #12 groceries, #3 in the include,
//         #5 from 2024), of which #20, #19, #3 are in this run
//     #21 SQ *BLUE BOTTLE 0001 BERKELEY (payee prefix → coffee 45%, weak)
//     #20, #19, #3 (Blue Bottle Oakland), #7 Corner Store 77 (no suggestion)
//   Recents: 17.

import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5098";
const FIXED_NOW = new Date(2026, 9, 30, 12, 0, 0); // local 2026-10-30, the fixture's TODAY
const COPY = resolve(import.meta.dirname, "../../target/e2e-review");

test.beforeEach(async ({page}) => {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            // insightsOpen: false — see keys.e2e.ts: the expanded charts squeeze
            // the table at this viewport.
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token, insightsOpen: false}));
        },
        [API_URL, API_TOKEN]
    );
});

const position = (page: Page) => page.getByTestId("review-position");
/**
 * A card by its transaction index. During an exit two cards are briefly in the
 * DOM, each with its own footer, so footer lookups go through this too. The
 * peeks beneath carry no index and no test ids.
 */
const cardOf = (page: Page, index: number) => page.locator(`[data-card-index="${index}"]`);

/** The transaction block in `file` whose header starts with `header`, up to the next blank line. */
function block(file: string, header: string): string {
    const text = readFileSync(resolve(COPY, file), "utf8").replaceAll("\r\n", "\n");
    const at = text.indexOf(`\n${header}`);
    expect(at, `no transaction "${header}" in ${file}`).toBeGreaterThanOrEqual(0);
    const end = text.indexOf("\n\n", at + 1);
    return text.slice(at + 1, end < 0 ? undefined : end);
}

test("review unknowns from the keyboard: suggestion, similar, skip and back — and the bytes on disk", async ({page}) => {
    await page.goto("/");
    await expect(page.locator("tbody tr[data-txn]").first()).toBeVisible();
    await expect(page.getByTestId("review-unknowns-count")).toHaveText("8");
    await expect(page.getByTestId("review-recents-count")).toHaveText("17");
    // An unknown in the window wears the amber stripe.
    await expect(page.locator("tr[data-txn='22']")).toHaveAttribute("data-review-marker", "unknown");

    // 1. Open with `u`. Focus is on the card, and Reviewed waits for a category.
    await page.keyboard.press("u");
    await expect(page.getByTestId("review-overlay")).toBeVisible();
    await expect(position(page)).toHaveText("1 of 8");
    await expect(cardOf(page, 25)).toBeFocused();
    await expect(cardOf(page, 25).getByTestId("review-payee")).toContainText("Venmo payment 8812");
    await expect(cardOf(page, 25).getByTestId("review-primary")).toBeDisabled();
    // The journal's `j` is blind under the modal layer: nothing moves.
    await page.keyboard.press("j");
    await expect(position(page)).toHaveText("1 of 8");

    // 2. Skip; ⌘⇧↵ (Ctrl⇧↵ elsewhere) takes the strong suggestion, marks
    //    reviewed, saves and advances — the chord reaches the page.
    await page.keyboard.press("n");
    await expect(position(page)).toHaveText("2 of 8");
    await expect(cardOf(page, 23).getByTestId("review-chip").first()).toHaveAttribute("aria-pressed", "true");
    await expect(cardOf(page, 23)).toBeFocused();
    await page.keyboard.press("ControlOrMeta+Shift+Enter");
    await expect(position(page)).toHaveText("3 of 8");

    // 3. Tab-accept the suggestion in the account field (category AND the
    //    rename precedent), back to card mode, open Similar, check all, ⌘↵.
    await expect(cardOf(page, 22)).toBeFocused();
    await page.keyboard.press("a");
    const account = cardOf(page, 22).getByLabel("Account for posting 1");
    await expect(account).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(account).toHaveValue("expenses:food:coffee");
    await expect(account).toBeFocused();
    await expect(cardOf(page, 22).getByLabel("Description")).toHaveValue("Blue Bottle Coffee");
    await page.keyboard.press("Escape");
    await expect(cardOf(page, 22)).toBeFocused();
    await expect(cardOf(page, 22).getByTestId("review-similar-count")).toHaveText("7");
    await page.keyboard.press("s");
    await page.keyboard.press("Shift+A");
    await expect(cardOf(page, 22).getByTestId("review-similar-all")).toBeChecked();
    await page.keyboard.press("ControlOrMeta+Enter");
    // D5a: #20, #19 and #3 were in this run and are categorised now — they
    // leave it, and the cursor lands past them on #21.
    await expect(position(page)).toHaveText("4 of 5");
    await expect(cardOf(page, 21)).toBeFocused();

    // 4. Skip one, come back with ←, Tab the (weak) suggestion in, `y`.
    await page.keyboard.press("n");
    await expect(position(page)).toHaveText("5 of 5");
    await expect(cardOf(page, 7).getByTestId("review-payee")).toContainText("Corner Store 77");
    await page.keyboard.press("ArrowLeft");
    await expect(position(page)).toHaveText("4 of 5");
    await expect(cardOf(page, 21)).toBeFocused();
    await page.keyboard.press("a");
    await page.keyboard.press("Tab");
    await expect(cardOf(page, 21).getByLabel("Account for posting 1")).toHaveValue("expenses:food:coffee");
    await page.keyboard.press("Escape");
    await expect(cardOf(page, 21).getByTestId("review-primary")).toContainText("Mark Reviewed and Save");
    await page.keyboard.press("y");
    await expect(position(page)).toHaveText("5 of 5");

    // Close (Corner Store is untouched, so no confirm). The cursor lands on
    // the last reviewed transaction, and the badges follow the writes.
    await expect(cardOf(page, 7)).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("review-overlay")).toBeHidden();
    await expect(page.getByTestId("review-unknowns-count")).toHaveText("2");
    await expect(page.locator("tr[aria-current='true']")).toContainText("BERKELEY");

    // 5. The bytes on disk.
    const nflx = block("review.journal", "2026-10-14 NFLX DIGITAL 8812");
    expect(nflx).toContain("reviewed:2026-10-30");
    expect(nflx).toMatch(/expenses:streaming\s+\$15\.49/);

    const card = block("review.journal", "2026-10-12 Blue Bottle Coffee");
    expect(card).toContain("reviewed:2026-10-30");
    expect(card).toContain("payee-key:sq blue bottle oakland");
    expect(card).toContain("receipt: 0042");
    expect(card).toMatch(/expenses:food:coffee\s+\$6\.75/);

    // Similar ones got the change, and are NOT marked reviewed.
    const sibling = block("review.journal", "2026-10-03 Blue Bottle Coffee");
    expect(sibling).toMatch(/expenses:food:coffee\s+\$5\.50/);
    expect(sibling).not.toContain("reviewed:");
    const groceries = block("review.journal", "2026-06-20 Blue Bottle Coffee");
    expect(groceries).toMatch(/expenses:food:groceries\s+\$12\.00/);
    expect(groceries).not.toContain("reviewed:");
    // …across both files of the journal, in one batch.
    const included = block("review-2025.journal", "2025-12-02 Blue Bottle Coffee");
    expect(included).toMatch(/expenses:food:coffee\s+\$5\.75/);

    const berkeley = block("review.journal", "2026-10-05 SQ *BLUE BOTTLE 0001 BERKELEY");
    expect(berkeley).toContain("reviewed:2026-10-30");
    expect(berkeley).toMatch(/expenses:food:coffee\s+\$4\.00/);

    // Skipped, so untouched; elided legs stayed elided.
    expect(block("review.journal", "2026-10-20 Venmo payment 8812")).toBe(
        "2026-10-20 Venmo payment 8812  ; reviewed:2026-10-21\n    expenses:unknown  $40.00\n    assets:checking"
    );
    expect(card).toMatch(/\n {4}liabilities:visa$/);
});
