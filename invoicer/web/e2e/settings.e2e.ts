// The Settings page end to end (SETTINGS-PLAN §4, §6), keyboard first: `g ,`,
// Cmd/Ctrl+, and the sidebar open it; Tab reaches its controls; Cmd/Ctrl+S
// saves only what changed into the books' ledgeline.toml, keeping its
// comments and other tables; a new invoice takes the saved default terms;
// "Use the default" takes the key out again. An outside edit of the file
// reloads the form silently, or shows the conflict notice under unsaved
// edits. A deposit-payments change is refused while a deposit waits for its
// final invoice (D5); a change that affects new bookings asks first, listing
// its effect. Appearance applies at once and the zoom keys step the scale.
// No setting's file name, `[[`, or `{` shows anywhere, and nothing overlaps
// at 480 px or at 200 %.
//
// Its own server: `--server` on :5196 over a scratch copy of
// invoicer/fixtures/cash (target/e2e-invoicer-settings/, staged fresh per run)
// with e2e/fixtures/settings-adatum.md added to the copy as
// customers/Adatum.md (playwright.config.ts). `vite preview` proxies only to
// :5199, so this spec loads the SPA the server embeds, as cash.e2e.ts does,
// and skips loudly on build.rs's placeholder (REQUIRE_REAL_SPA=1 makes that a
// failure). Fixture facts at the clock, 2026-11-17 (fixtures/README.md):
//   - the books are cash books; their ledgeline.toml opens with a comment,
//     has `[books] basis = "cash"`, and `[invoicer]` sets company,
//     revenue-account, deposit-account and number-start = 201, nothing else;
//   - 206 is a deposit invoice paid by wire, which booked to
//     revenue:consulting; 207, its final invoice, is unpaid: so changing
//     deposit payments is refused, naming 206;
//   - Adatum (staged) sets no terms and no template; the next number is 208.
//
// The server keeps UI prefs in the browser (LEDGELINE_UI_PREFS_IN_BROWSER):
// it answers the defaults and writes no prefs file, and the invoicer keeps
// nothing in the browser's storage but the first-paint cache. So a text size
// applies at once but can't be shown to survive a reload here, and no "Use
// for these books only" pin is offered.
//
// Serial: the tests save and edit the copy's ledgeline.toml, each putting it
// back as the fixture has it. Written with the Settings page and NOT run by
// the agent that wrote it (agents never run Playwright); the lead runs it.

import {readFileSync, writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Locator, type Page} from "@playwright/test";
import {SETTING_KEYS, WORD_KEYS} from "../src/lib/settings";
import {API_TOKEN, SETTINGS_COPY, SETTINGS_SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0); // local 2026-11-17, the fixture's clock
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const BOOKS = resolve(import.meta.dirname, "..", SETTINGS_COPY);
const SETTINGS_FILE = resolve(BOOKS, "ledgeline.toml");
/** The settings file as committed: every test leaves the copy like this. */
const ORIGINAL = readFileSync(resolve(import.meta.dirname, "..", "..", "fixtures", "cash", "ledgeline.toml"), "utf8");
/** How long a change on disk may take to reach the page (the watcher, then a refetch). */
const ON_DISK_MS = 10_000;

const settingsText = (): string => readFileSync(SETTINGS_FILE, "utf8");
const read = (file: string): string => readFileSync(resolve(BOOKS, file), "utf8");

/**
 * Default terms of 45 days in the copy's settings file, as the Save test
 * writes them (afterEach puts the file back, so a test that needs them sets
 * them itself), once the page shows the server has read them.
 */
async function withDefaultTerms(page: Page): Promise<void> {
    writeFileSync(SETTINGS_FILE, `${ORIGINAL}default-terms = 45\n`);
    await openSettings(page);
    await expect(page.getByTestId("input-default-terms")).toHaveValue("45", {timeout: ON_DISK_MS});
}

/**
 * What the page must never show: a setting by its name in the file (the
 * hyphenated ones and `filename`; the others are plain words), a shared
 * matching key, or the file's own vocabulary.
 */
const RAW_NAMES = [
    ...SETTING_KEYS.filter((k) => k.includes("-")),
    "filename",
    ...WORD_KEYS.filter((k) => k.includes("-")),
    "settlementLookbackDays",
    "settlementToleranceBps",
    "settlementLeadDays",
    "uiScale",
    "ledgeline.toml",
    "[invoicer",
    "[[",
    "{",
];

test.use({baseURL: SETTINGS_SERVER_URL});
test.describe.configure({mode: "serial"});

/** Open `path` in the SPA the server embeds, with the launch token; skip when it is the placeholder. */
async function open(page: Page, path: string): Promise<void> {
    const shell = await page.request.get(`${SETTINGS_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server on :5196 embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer settings.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${SETTINGS_SERVER_URL}/app${path}#token=${API_TOKEN}`);
    await expect(page).toHaveURL(new RegExp(`/app${path.replace(/[?]/g, "\\?")}$`));
}

/** Open the Settings page's These books tab, loaded. */
async function openSettings(page: Page): Promise<void> {
    await open(page, "/settings");
    await expect(page.getByTestId("settings-books")).toBeVisible();
}

/** A two-key chord (`g ,`). */
async function chord(page: Page, first: string, second: string): Promise<void> {
    await page.keyboard.press(first);
    await page.keyboard.press(second);
}

/** Press Tab until `target` has focus; fails after `max` presses. */
async function tabTo(page: Page, target: Locator, max = 120): Promise<void> {
    for (let i = 0; i < max && !(await target.evaluate((el) => el === document.activeElement)); i++) await page.keyboard.press("Tab");
    await expect(target).toBeFocused();
}

/** Save with Cmd/Ctrl+S and wait for "Saved.". */
async function saveByKey(page: Page): Promise<void> {
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("settings-saved")).toHaveText("Saved.");
    await expect(page.getByTestId("settings-actions")).toHaveCount(0);
}

/** The live px per rem. */
const rootPx = (page: Page) => page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize));

/**
 * Every pair of visible controls on the page whose boxes overlap (by more
 * than a pixel each way), and every setting row whose label side and
 * control side overlap. A control drawn inside another (a combobox's toggle
 * in its field) is by design, so containment doesn't count.
 */
async function overlaps(page: Page): Promise<string[]> {
    return page.getByTestId("settings-page").evaluate((root) => {
        const name = (el: Element): string =>
            (el as HTMLElement).dataset.testid ?? el.getAttribute("aria-label") ?? (el.textContent ?? "").trim().slice(0, 30) ?? el.tagName;
        const shown = (el: Element): boolean => {
            const r = el.getBoundingClientRect();
            return r.width > 1 && r.height > 1 && el.checkVisibility({checkOpacity: true, checkVisibilityCSS: true});
        };
        const cross = (a: DOMRect, b: DOMRect): boolean =>
            Math.min(a.right, b.right) - Math.max(a.left, b.left) > 1 && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 1;
        const holds = (o: DOMRect, i: DOMRect): boolean =>
            o.left <= i.left + 0.5 && o.right >= i.right - 0.5 && o.top <= i.top + 0.5 && o.bottom >= i.bottom - 0.5;
        const found: string[] = [];
        const controls = Array.from(root.querySelectorAll("input, select, textarea, button, a[href], summary")).filter(shown);
        const boxes = controls.map((el) => el.getBoundingClientRect());
        for (let i = 0; i < controls.length; i++) {
            for (let j = i + 1; j < controls.length; j++) {
                const [a, b] = [boxes[i], boxes[j]];
                if (cross(a, b) && !holds(a, b) && !holds(b, a)) found.push(`${name(controls[i])} × ${name(controls[j])}`);
            }
        }
        for (const row of Array.from(root.querySelectorAll('[data-testid^="setting-"].grid')).filter(shown)) {
            const [label, control] = Array.from(row.children);
            if (label !== undefined && control !== undefined && cross(label.getBoundingClientRect(), control.getBoundingClientRect())) {
                found.push(`${name(row)}: its label and its control`);
            }
        }
        return found;
    });
}

/** Nothing on the page is wider than the window, and no controls overlap. */
async function expectNoOverlap(page: Page, width: number): Promise<void> {
    const main = page.locator("main");
    // From the top: the sticky header (tabs, Save) covers what scrolls under
    // it by design, which is not an overlap.
    await main.evaluate((el) => el.scrollTo(0, 0));
    await page.evaluate(() => window.scrollTo(0, 0));
    expect(await main.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
    const box = await page.getByTestId("settings-page").boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x + box!.width).toBeLessThanOrEqual(width);
    expect(await overlaps(page)).toEqual([]);
}

test.describe("Settings (:5196, cash books)", () => {
    test.afterEach(() => {
        // Whatever a failure left, the next test starts from the fixture's file.
        if (settingsText() !== ORIGINAL) writeFileSync(SETTINGS_FILE, ORIGINAL);
    });

    test("g , and Cmd/Ctrl+, open Settings, and so does the sidebar link", async ({page}) => {
        await open(page, "/");
        await chord(page, "g", ",");
        await expect(page).toHaveURL(/\/app\/settings$/);
        await expect(page.getByRole("heading", {name: "Settings", exact: true})).toBeVisible();
        await expect(page.getByTestId("settings-tab-books")).toHaveAttribute("aria-selected", "true");

        await chord(page, "g", "i");
        await expect(page).toHaveURL(/\/app\/invoices$/);
        // From inside a field too.
        await page.keyboard.press("/");
        await page.keyboard.press("ControlOrMeta+Comma");
        await expect(page).toHaveURL(/\/app\/settings$/);

        await chord(page, "g", "d");
        await expect(page).toHaveURL(/\/app\/?$/);
        const link = page.getByTestId("sidebar-settings");
        await link.focus();
        await page.keyboard.press("Enter");
        await expect(page).toHaveURL(/\/app\/settings$/);
        await expect(link).toHaveAttribute("aria-current", "page");

        // The ? sheet lists both.
        await page.keyboard.press("?");
        await expect(page.getByTestId("key-help")).toContainText("Go to Settings");
        await expect(page.getByTestId("key-help")).toContainText("Open Settings");
        await page.keyboard.press("Escape");
    });

    test("the form as the file has it: plain labels, Default and Use the default, the tabs by arrow keys", async ({page}) => {
        await openSettings(page);
        // Set in the file: its value, and Use the default. Unset: the Default badge.
        await expect(page.getByTestId("input-number-start")).toHaveValue("201");
        await expect(page.getByTestId("setting-number-start-use-default")).toBeVisible();
        await expect(page.getByTestId("setting-default-terms-default")).toHaveText("Default");
        await expect(page.getByTestId("setting-default-terms-use-default")).toHaveCount(0);
        await expect(page.getByTestId("setting-dso-days-default")).toBeVisible();
        await expect(page.getByTestId("input-company")).toHaveValue("[[My Company]]");
        await expect(page.getByTestId("input-company").locator("option:checked")).toHaveText("My Company");
        // The books are Ledgeline's, shown in words; booking settings are marked.
        await expect(page.getByTestId("books-basis")).toHaveText("Cash");
        await expect(page.getByTestId("setting-payment-allocation-affects")).toHaveText("Affects new bookings");
        await expect(page.getByTestId("setting-cash-deposits-affects")).toBeVisible();
        await expect(page.getByTestId("setting-dso-days-affects")).toHaveCount(0);
        // The coming-later choices are there, and can't be chosen.
        await expect(page.getByTestId("coming-oldest-row-first")).toBeDisabled();
        await expect(page.getByTestId("coming-row-first-open-day")).toContainText("Coming later");
        // Nothing unsaved, so no Save.
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);

        // The tabs move with the arrow keys; the form's state stays.
        const books = page.getByTestId("settings-tab-books");
        await books.focus();
        await page.keyboard.press("ArrowRight");
        await expect(page).toHaveURL(/\/app\/settings\?tab=appearance$/);
        await expect(page.getByTestId("settings-tab-appearance")).toBeFocused();
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        await page.keyboard.press("Home");
        await expect(page).toHaveURL(/\/app\/settings\?tab=books$/);
        await expect(books).toBeFocused();
        expect(settingsText()).toBe(ORIGINAL);
    });

    test("Tab reaches the controls; Cmd/Ctrl+S saves default terms and keeps the file's comments and tables", async ({page}) => {
        await openSettings(page);
        const terms = page.getByTestId("input-default-terms");
        // Past the sidebar and the tabs, the company comes first, then New invoices.
        await tabTo(page, page.getByTestId("input-company"));
        await tabTo(page, terms, 40);
        await page.keyboard.type("45");
        await expect(page.getByTestId("setting-default-terms-changed")).toHaveText("Changed");
        await expect(page.getByTestId("settings-save")).toBeVisible();
        await expect(page.getByTestId("settings-discard")).toHaveText("Discard changes");
        // Nothing is written before Save.
        expect(settingsText()).toBe(ORIGINAL);

        await saveByKey(page);
        const text = settingsText();
        expect(text).toMatch(/^default-terms = 45$/m);
        // Everything else exactly as it was: the opening comment, [books], the other keys.
        expect(text.startsWith('# Ledgeline settings for the invoicer\'s cash fixture (synthetic).\n\n[books]\nbasis = "cash"\n')).toBe(true);
        for (const line of ORIGINAL.split("\n")) expect(text).toContain(line);
        expect(text.replace(/^default-terms = 45\n/m, "")).toBe(ORIGINAL);
        await expect(page.getByTestId("setting-default-terms-use-default")).toBeVisible();
        await expect(page.getByTestId("setting-default-terms-default")).toHaveCount(0);

        // A reload shows it saved.
        await page.reload();
        await expect(page.getByTestId("input-default-terms")).toHaveValue("45");
    });

    test("a new invoice for a customer with no terms gets the default terms", async ({page}) => {
        await withDefaultTerms(page);
        await open(page, "/invoices");
        await page.keyboard.press("n");
        await expect(page).toHaveURL(/\/app\/new$/);
        const customer = page.getByTestId("new-customer");
        await expect(customer).toBeFocused();
        await page.keyboard.type("Adatum");
        await expect(page.getByRole("listbox", {name: "Customer suggestions"}).getByRole("option", {name: /^Adatum/})).toBeVisible();
        await page.keyboard.press("Enter");
        await expect(customer).toHaveValue("Adatum");
        await page.getByTestId("new-create").click();

        await expect(page).toHaveURL(/\/app\/invoices\/\d+\/edit$/);
        const number = /\/invoices\/(\d+)\/edit$/.exec(page.url())?.[1] ?? "";
        expect(number).toBe("208");
        const file = read(`invoices/2026-11-17 Adatum Invoice ${number}.md`);
        expect(file).toMatch(/^terms: 45$/m);
        expect(file).toMatch(/^due_date: 2027-01-01$/m);
    });

    test("Use the default removes the key, leaving the file as it was", async ({page}) => {
        await withDefaultTerms(page);
        const useDefault = page.getByTestId("setting-default-terms-use-default");
        await useDefault.focus();
        await page.keyboard.press("Enter");
        await expect(page.getByTestId("input-default-terms")).toHaveValue("");
        await expect(page.getByTestId("setting-default-terms-default")).toBeVisible();
        await expect(page.getByTestId("settings-save")).toBeVisible();

        await saveByKey(page);
        expect(settingsText()).not.toContain("default-terms");
        expect(settingsText()).toBe(ORIGINAL);
    });

    test("an outside edit reloads the form silently; under unsaved edits it shows the conflict notice", async ({page}) => {
        await openSettings(page);
        const dso = page.getByTestId("input-dso-days");
        await expect(dso).toHaveValue("90");

        // No edits pending: the page takes the file as it now is, and says nothing.
        writeFileSync(SETTINGS_FILE, `${ORIGINAL}dso-days = 60\n`);
        await expect(dso).toHaveValue("60", {timeout: ON_DISK_MS});
        await expect(page.getByTestId("conflict-notice")).toHaveCount(0);
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);
        await expect(page.getByTestId("setting-dso-days-use-default")).toBeVisible();

        // Edits pending: they stay, and the notice says the file changed.
        const paid = page.getByTestId("input-paid-window-days");
        await paid.focus();
        await page.keyboard.press("ControlOrMeta+a");
        await page.keyboard.type("45");
        await expect(page.getByTestId("settings-save")).toBeVisible();
        writeFileSync(SETTINGS_FILE, `${ORIGINAL}dso-days = 75\n`);
        const notice = page.getByTestId("conflict-notice");
        await expect(notice).toBeVisible({timeout: ON_DISK_MS});
        await expect(notice).toContainText("This settings file changed on disk while you were editing it.");
        await expect(paid).toHaveValue("45");
        await expect(dso).toHaveValue("60");
        // Save is refused until you choose; nothing is written.
        await page.keyboard.press("ControlOrMeta+s");
        await expect(page.getByTestId("settings-save-error")).toContainText("changed outside the app");
        expect(settingsText()).toBe(`${ORIGINAL}dso-days = 75\n`);

        // Reload theirs: their value, mine dropped.
        await page.getByTestId("conflict-reload").focus();
        await page.keyboard.press("Enter");
        await expect(notice).toHaveCount(0);
        await expect(dso).toHaveValue("75");
        await expect(paid).toHaveValue("30");
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);

        // Put the fixture's file back; the page follows, silently.
        writeFileSync(SETTINGS_FILE, ORIGINAL);
        await expect(dso).toHaveValue("90", {timeout: ON_DISK_MS});
        await expect(page.getByTestId("conflict-notice")).toHaveCount(0);
    });

    test("deposit payments can't change while 206's deposit waits for its final invoice", async ({page}) => {
        await openSettings(page);
        const now = page.getByTestId("choice-cash-deposits-income-on-receipt");
        await expect(now).toBeChecked();
        await now.focus();
        await page.keyboard.press("ArrowDown");
        await expect(page.getByTestId("choice-cash-deposits-liability-until-final")).toBeChecked();

        await page.keyboard.press("ControlOrMeta+s");
        const refused = page.getByTestId("settings-refused");
        await expect(refused).toBeVisible();
        await expect(refused.getByRole("heading", {name: "This change can't be saved yet", exact: true})).toBeVisible();
        await expect(refused).toContainText("Deposit payments:");
        await expect(page.getByTestId("settings-refused-deposits")).toContainText("Deposit invoice 206 for Customer X");
        await expect(page.getByTestId("settings-refused-deposits")).toContainText("$5,000.00 waiting");
        // No confirming past it: the choices are to undo the change or keep editing.
        await expect(page.getByTestId("settings-refused-confirm")).toHaveText("Undo this change");
        await expect(refused.getByRole("button", {name: "Keep editing", exact: true})).toBeFocused();
        expect(settingsText()).toBe(ORIGINAL);

        await page.keyboard.press("Tab");
        await expect(page.getByTestId("settings-refused-confirm")).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(refused).toHaveCount(0);
        await expect(now).toBeChecked();
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);
        expect(settingsText()).toBe(ORIGINAL);
    });

    test("a change that affects new bookings asks first, listing its effect", async ({page}) => {
        await openSettings(page);
        const matchRows = page.getByTestId("choice-payment-allocation-match-rows");
        await expect(matchRows).toBeChecked();
        await matchRows.focus();
        await page.keyboard.press("ArrowDown");
        await expect(page.getByTestId("choice-payment-allocation-pro-rata")).toBeChecked();

        await page.keyboard.press("ControlOrMeta+s");
        const confirm = page.getByTestId("settings-confirm");
        await expect(confirm).toBeVisible();
        await expect(confirm.getByRole("heading", {name: "Change how new bookings are made?", exact: true})).toBeVisible();
        const effects = page.getByTestId("settings-effects");
        await expect(effects.getByRole("listitem")).toHaveCount(1);
        await expect(effects).toContainText("Splitting a payment over rows:");
        await expect(effects).toContainText("Payments you apply from now on are split pro rata.");
        await expect(effects).toContainText("Payments already in your books keep the split they were booked with.");
        await expect(effects).not.toContainText("payment-allocation");
        expect(settingsText()).toBe(ORIGINAL);

        // Cancel (the focus starts there) keeps the edit and writes nothing.
        await page.keyboard.press("Escape");
        await expect(confirm).toHaveCount(0);
        await expect(page.getByTestId("settings-save")).toBeVisible();
        expect(settingsText()).toBe(ORIGINAL);

        // Again, and confirm by Tab and Enter.
        await page.keyboard.press("ControlOrMeta+s");
        await expect(confirm).toBeVisible();
        await page.keyboard.press("Tab");
        await expect(page.getByTestId("settings-confirm-confirm")).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page.getByTestId("settings-saved")).toHaveText("Saved.");
        expect(settingsText()).toMatch(/^payment-allocation = "pro-rata"$/m);

        // Back to the default: that is a booking change too, so it asks again.
        await page.getByTestId("setting-payment-allocation-use-default").focus();
        await page.keyboard.press("Enter");
        await page.keyboard.press("ControlOrMeta+s");
        await expect(confirm).toBeVisible();
        await expect(page.getByTestId("settings-effects")).toContainText("move each row toward its invoiced amount");
        await page.getByTestId("settings-confirm-confirm").click();
        await expect(page.getByTestId("settings-saved")).toHaveText("Saved.");
        expect(settingsText()).toBe(ORIGINAL);
    });

    test("no setting's file name, [[ or { shows on either tab", async ({page}) => {
        await openSettings(page);
        // The folded Wording section too, opened from the keyboard.
        const wording = page.getByTestId("settings-wording");
        await wording.locator("summary").focus();
        await page.keyboard.press("Enter");
        await expect(wording).toHaveAttribute("open", "");
        await expect(page.getByTestId("setting-words-customer-tag")).toBeVisible();

        const pageText = page.getByTestId("settings-page");
        const shown = await pageText.innerText();
        for (const raw of RAW_NAMES) expect(shown, `the page shows ${raw}`).not.toContain(raw);
        // Nor in what the text fields hold (the file name is pieces and chips).
        const typed = await pageText.locator('input[type="text"]:visible').evaluateAll((inputs) => inputs.map((i) => (i as HTMLInputElement).value));
        for (const value of typed) {
            expect(value).not.toContain("{");
            expect(value).not.toContain("[[");
        }
        // The next number, so 209 once the test above has added 208.
        await expect(page.getByTestId("filename-example")).toHaveText(/^2026-11-17 Acme Invoice \d+\.md$/);

        await page.keyboard.press("Escape");
        await page.keyboard.press("2");
        await expect(page).toHaveURL(/\/app\/settings\?tab=appearance$/);
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        const appearance = await pageText.innerText();
        for (const raw of RAW_NAMES) expect(appearance, `the Appearance tab shows ${raw}`).not.toContain(raw);
    });

    test("Appearance applies at once, with no Save; the zoom keys step the scale", async ({page}) => {
        await open(page, "/settings?tab=appearance");
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        await expect(page.getByTestId("appearance-shared")).toContainText("Shared with Ledgeline");
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);
        expect(await rootPx(page)).toBe(16);

        const scale = page.getByTestId("appearance-scale");
        await scale.focus();
        await scale.selectOption("125");
        await expect.poll(() => rootPx(page)).toBe(20);
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);
        // Not asserted: that 125 % survives a reload. In browser mode the
        // server answers the defaults and nothing keeps the choice (see the top).

        // The zoom keys, from anywhere, and the control follows them.
        await page.keyboard.press("Escape");
        await page.keyboard.press("ControlOrMeta+Equal");
        await expect.poll(() => rootPx(page)).toBe(24);
        await expect(scale).toHaveValue("150");
        await page.keyboard.press("ControlOrMeta+Minus");
        await expect.poll(() => rootPx(page)).toBe(20);
        await page.keyboard.press("ControlOrMeta+Digit0");
        await expect.poll(() => rootPx(page)).toBe(16);
        await expect(scale).toHaveValue("100");

        // A theme applies at once too.
        await page.getByTestId("theme-card-nord").getByRole("radio").click();
        await expect(page.locator("html")).toHaveAttribute("data-theme", "nord");
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);
        expect(settingsText()).toBe(ORIGINAL);
    });

    test("no 'Use for these books only' pin in browser mode", async ({page}) => {
        await openSettings(page);
        for (const key of ["settlementLookbackDays", "settlementToleranceBps", "settlementLeadDays"]) {
            await expect(page.getByTestId(`pin-${key}`)).toHaveCount(0);
        }
        await expect(page.getByTestId("settings-matching-shared")).toContainText("shared with Ledgeline");
        await expect(page.getByTestId("settings-matching-shared")).not.toContainText("these books only");
        await page.keyboard.press("2");
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        await expect(page.getByTestId("appearance-pin-theme")).toHaveCount(0);
        await expect(page.getByTestId("appearance-pin-scale")).toHaveCount(0);
        await expect(page.getByText("Use for these books only")).toHaveCount(0);
    });

    test("fits a 480 px window: nothing wider than it, no controls overlap", async ({page}) => {
        await page.setViewportSize({width: 480, height: 900});
        await openSettings(page);
        // Below lg the sections are a wrapping row of links, not a rail.
        await expect(page.getByTestId("settings-rail")).toBeHidden();
        await expect(page.getByTestId("settings-section-row")).toBeVisible();
        await page.getByTestId("settings-wording").locator("summary").click();
        await expectNoOverlap(page, 480);
        // With unsaved changes, Save and Discard join the header and still fit.
        await page.getByTestId("input-dso-days").fill("60");
        await expect(page.getByTestId("settings-save")).toBeVisible();
        await expectNoOverlap(page, 480);
        await page.getByTestId("settings-discard").click();
        await page.keyboard.press("2");
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        await expectNoOverlap(page, 480);
    });

    test("fits at 200 %: nothing wider than the window, no controls overlap", async ({page}) => {
        // The saved text size, as the server would answer it outside browser mode.
        await page.route("**/api/ui-prefs", async (route) => {
            if (route.request().method() !== "GET") return route.continue();
            const response = await route.fetch();
            const body = (await response.json()) as {prefs?: Record<string, unknown>};
            await route.fulfill({response, json: {...body, prefs: {...body.prefs, uiScale: 200}}});
        });
        await page.setViewportSize({width: 1280, height: 900});
        await openSettings(page);
        await expect.poll(() => rootPx(page)).toBe(32);
        await page.getByTestId("settings-wording").locator("summary").click();
        await expectNoOverlap(page, 1280);
        await page.getByTestId("input-dso-days").fill("60");
        await expect(page.getByTestId("settings-save")).toBeVisible();
        await expectNoOverlap(page, 1280);
        await page.getByTestId("settings-discard").click();
        await page.keyboard.press("2");
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        await expectNoOverlap(page, 1280);
    });
});
