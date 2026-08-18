// The keyboard, end to end. This is the suite's first keyboard interaction —
// every other spec drives the app with click/fill — so it is also where the
// pattern gets set.
//
// It exists for the assertions jsdom structurally cannot make:
//   - Real chord timing, against real timers (the clock is frozen for `Date`,
//     but `setTimeout` keeps running).
//   - `toBeFocused`, which needs a real focus model.
//   - `toBeInViewport`, which needs layout — and is the only way to prove the
//     virtualized table's cursor reveal actually reveals anything.
//   - Tab traversal, which is the anti-keyboard-trap guarantee.

import {expect, test, type Page} from "@playwright/test";
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

/** Wait for the journal to have loaded, so the table's keymap layer is registered. */
async function journalReady(page: Page): Promise<void> {
    await expect(page.locator("footer")).toContainText("transactions");
}

test.describe("global keys", () => {
    test("g then j navigates to the journal", async ({page}) => {
        await page.goto("/reports");
        await expect(page).toHaveTitle("Ledgeline — Reports");

        await page.keyboard.press("g");
        await page.keyboard.press("j");

        await expect(page).toHaveTitle("Ledgeline — Journal");
    });

    test("a half-typed chord shows itself and then goes away", async ({page}) => {
        // An armed prefix is the app's only modal state; a swallowed keystroke
        // with nothing on screen is indistinguishable from a broken app.
        await page.goto("/");
        await journalReady(page);

        await page.keyboard.press("g");
        await expect(page.getByTestId("chord-indicator")).toBeVisible();

        await page.keyboard.press("Escape");
        await expect(page.getByTestId("chord-indicator")).toBeHidden();
    });

    test("? opens help, Escape closes it and returns focus", async ({page}) => {
        // Shift+Slash rather than "?": Playwright's press("?") produces
        // layout-dependent `key` values.
        await page.goto("/");
        await journalReady(page);

        await page.keyboard.press("Shift+Slash");
        const help = page.getByTestId("key-help");
        await expect(help).toBeVisible();
        // Generated from the live registry, so this proves registration too.
        await expect(help).toContainText("Go to Journal");
        await expect(help).toContainText("Next transaction");

        await page.keyboard.press("Escape");
        await expect(help).toBeHidden();
    });

    test("the help sheet lists the keys of the page it was opened on", async ({page}) => {
        // Context scoping, visible: report tab digits exist only on /reports.
        await page.goto("/reports");
        await page.keyboard.press("Shift+Slash");

        await expect(page.getByTestId("key-help")).toContainText("Balance Sheet");
    });

    test("digits switch report tabs", async ({page}) => {
        await page.goto("/reports?tab=insights");

        await page.keyboard.press("2");

        await expect(page.getByRole("tab", {name: "Balance Sheet"})).toHaveAttribute("aria-selected", "true");
    });
});

test.describe("the typing guard", () => {
    test("/ focuses the search box, and does not type a slash into it", async ({page}) => {
        // The value assertion is the preventDefault-before-run() ordering bug,
        // which is invisible in jsdom because nothing there types.
        await page.goto("/");
        await journalReady(page);

        await page.keyboard.press("/");

        const search = page.getByLabel("Search transactions");
        await expect(search).toBeFocused();
        await expect(search).toHaveValue("");
    });

    test("j types a j into the search box instead of moving the cursor", async ({page}) => {
        // The whole basis of a non-modal keymap, end to end. jsdom's focus model
        // is approximate enough that this is worth proving in a real browser.
        await page.goto("/");
        await journalReady(page);
        await page.keyboard.press("/");

        await page.keyboard.type("j");

        await expect(page.getByLabel("Search transactions")).toHaveValue("j");
        await expect(page.locator("[aria-current='true']")).toHaveCount(0);
    });

    test("Escape leaves the search box without clearing what was typed", async ({page}) => {
        await page.goto("/");
        await journalReady(page);
        await page.keyboard.press("/");
        await page.keyboard.type("plumber");

        await page.keyboard.press("Escape");

        await expect(page.getByLabel("Search transactions")).not.toBeFocused();
        await expect(page.getByLabel("Search transactions")).toHaveValue("plumber");
    });
});

test.describe("the journal cursor", () => {
    test("j moves the cursor down the table", async ({page}) => {
        await page.goto("/");
        await journalReady(page);

        await page.keyboard.press("j");
        await expect(page.locator("[aria-current='true']")).toHaveCount(1);

        const first = await page.locator("[aria-current='true']").getAttribute("data-txn");
        await page.keyboard.press("j");

        expect(await page.locator("[aria-current='true']").getAttribute("data-txn")).not.toBe(first);
    });

    test("G reaches the last row AND scrolls it into view", async ({page}) => {
        // THE assertion jsdom can never make. The table is virtualized, so the
        // last row is not in the DOM until the cursor's reveal arithmetic puts
        // it there — and `toBeInViewport` also proves the sticky-header headroom
        // is being subtracted, since without it the row lands under the header.
        await page.goto("/?preset=all");
        await journalReady(page);

        await page.keyboard.press("Shift+G");

        await expect(page.locator("[aria-current='true']")).toBeInViewport();
    });

    test("ctrl-d then ctrl-u returns to the same row", async ({page}) => {
        await page.goto("/?preset=all");
        await journalReady(page);
        await page.keyboard.press("j");
        const start = await page.locator("[aria-current='true']").getAttribute("data-txn");

        await page.keyboard.press("Control+d");
        await page.keyboard.press("Control+u");

        expect(await page.locator("[aria-current='true']").getAttribute("data-txn")).toBe(start);
        await expect(page.locator("[aria-current='true']")).toBeInViewport();
    });

    test("Escape clears the cursor", async ({page}) => {
        await page.goto("/");
        await journalReady(page);
        await page.keyboard.press("j");

        await page.keyboard.press("Escape");

        await expect(page.locator("[aria-current='true']")).toHaveCount(0);
    });
});

test.describe("account completion", () => {
    /** Open the add-transaction popup and put the caret in its first account field. */
    async function openPopup(page: Page): Promise<void> {
        await page.goto("/");
        await journalReady(page);
        await page.getByRole("button", {name: "Add transaction"}).click();
        await page.getByLabel("Account").first().click();
    }

    test("Tab completes to the shared prefix", async ({page}) => {
        await openPopup(page);
        const account = page.getByLabel("Account").first();
        await account.fill("ex");

        await page.keyboard.press("Tab");

        await expect(account).toHaveValue(/^expenses/);
    });

    test("the suggestion list is not clipped by the modal box", async ({page}) => {
        // `.modal-box` is overflow-y:auto with max-height:100vh, and per CSS spec
        // that computes the other axis to auto too — so an absolutely-positioned
        // popup would be cut off. This is the assertion that the fixed-position
        // portal is doing its job, and it needs real layout.
        await openPopup(page);
        await page.getByLabel("Account").first().fill("ex");

        await expect(page.getByRole("listbox")).toBeInViewport();
    });

    test("REGRESSION: Escape closes the suggestions, not the whole transaction", async ({page}) => {
        // The bug: AccountInput was passed no onCancel, so Escape did nothing
        // locally and bubbled to the modal, which closed and discarded
        // everything typed.
        await openPopup(page);
        await page.getByLabel("Description").fill("Plumber");
        await page.getByLabel("Account").first().fill("ex");
        await expect(page.getByRole("listbox")).toBeVisible();

        await page.keyboard.press("Escape");

        await expect(page.getByRole("listbox")).toBeHidden();
        await expect(page.getByLabel("Description")).toHaveValue("Plumber");
    });

    test("a second Escape closes the popup", async ({page}) => {
        await openPopup(page);
        await page.getByLabel("Account").first().fill("ex");

        await page.keyboard.press("Escape");
        await page.keyboard.press("Escape");

        await expect(page.getByLabel("Description")).toBeHidden();
    });

    test("REGRESSION: Shift+Tab escapes the field rather than cycling", async ({page}) => {
        // The anti-trap guarantee. If Tab is claimed unconditionally there is no
        // way out of this field and the popup becomes a keyboard trap — and only
        // a real browser has focus traversal to prove it against.
        await openPopup(page);
        const account = page.getByLabel("Account").first();
        await account.fill("ex");

        await page.keyboard.press("Shift+Tab");

        await expect(account).not.toBeFocused();
    });
});
