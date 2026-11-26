// The Settings page over books with no settings file (SETTINGS-PLAN D1, §3
// rule 6): opening the page writes nothing, and Save asks "These books have
// no settings file yet. Create it?" before it creates one. Cancel leaves
// the books without a file and the edit unsaved; confirming writes a file
// that opens with the header comment and holds the one changed setting.
//
// Its own server: `--server` on :5195 over a scratch copy of
// invoicer/fixtures/empty with the copy's ledgeline.toml deleted
// (target/e2e-invoicer-no-settings/, staged fresh per run;
// playwright.config.ts). Like empty.e2e.ts, it loads the SPA the server
// embeds and skips loudly on build.rs's placeholder (REQUIRE_REAL_SPA=1 makes
// that a failure). The server keeps UI prefs in the browser, so nothing but
// Save can create the file.
//
// Serial: the second test creates the file the first one checks is absent.
// Written with the Settings page and NOT run by the agent that wrote it
// (agents never run Playwright); the lead runs it.

import {existsSync, readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, NEW_FILE_COPY, NEW_FILE_SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const SETTINGS_FILE = resolve(import.meta.dirname, "..", NEW_FILE_COPY, "ledgeline.toml");
/** The first line of the header a created settings file opens with (invoicer-core's FILE_HEADER). */
const HEADER_START = "# Settings for these books. Ledgeline Invoicer keeps its settings under\n";

test.use({baseURL: NEW_FILE_SERVER_URL});
test.describe.configure({mode: "serial"});

async function openSettings(page: Page): Promise<void> {
    const shell = await page.request.get(`${NEW_FILE_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server on :5195 embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer settings-new-file.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.clock.setFixedTime(FIXED_NOW);
    await page.goto(`${NEW_FILE_SERVER_URL}/app/settings#token=${API_TOKEN}`);
    await expect(page).toHaveURL(/\/app\/settings$/);
    await expect(page.getByTestId("settings-books")).toBeVisible();
}

test.describe("Settings with no settings file (:5195)", () => {
    test("opening the page writes nothing; every setting is its default", async ({page}) => {
        await openSettings(page);
        await expect(page.getByTestId("settings-no-file")).toContainText("These books have no settings file yet");
        await expect(page.getByTestId("setting-dso-days-default")).toBeVisible();
        await expect(page.getByTestId("setting-number-start-default")).toBeVisible();
        await expect(page.getByTestId("settings-read-only")).toHaveCount(0);
        await page.getByTestId("settings-tab-appearance").click();
        await expect(page.getByTestId("settings-appearance")).toBeVisible();
        expect(existsSync(SETTINGS_FILE)).toBe(false);
    });

    test("Save asks before it creates the file; confirming writes it with the header", async ({page}) => {
        await openSettings(page);
        const dso = page.getByTestId("input-dso-days");
        await dso.focus();
        await page.keyboard.press("ControlOrMeta+a");
        await page.keyboard.type("60");
        await expect(page.getByTestId("settings-save")).toBeVisible();

        // Asked, with the focus on Cancel; Escape cancels and writes nothing.
        await page.keyboard.press("ControlOrMeta+s");
        const ask = page.getByTestId("settings-create-file");
        await expect(ask).toBeVisible();
        await expect(ask.getByRole("heading", {name: "Create a settings file?", exact: true})).toBeVisible();
        await expect(ask).toContainText("These books have no settings file yet. Create it?");
        await expect(ask.getByRole("button", {name: "Cancel", exact: true})).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(ask).toHaveCount(0);
        await expect(page.getByTestId("settings-save")).toBeVisible();
        await expect(dso).toHaveValue("60");
        expect(existsSync(SETTINGS_FILE)).toBe(false);

        // Again, and confirm from the keyboard.
        await page.keyboard.press("ControlOrMeta+s");
        await expect(ask).toBeVisible();
        await page.keyboard.press("Tab");
        const create = page.getByTestId("settings-create-file-confirm");
        await expect(create).toHaveText("Create it and save");
        await expect(create).toBeFocused();
        await page.keyboard.press("Enter");
        await expect(page.getByTestId("settings-saved")).toHaveText("Saved.");
        await expect(page.getByTestId("settings-actions")).toHaveCount(0);

        expect(existsSync(SETTINGS_FILE)).toBe(true);
        const text = readFileSync(SETTINGS_FILE, "utf8");
        expect(text.startsWith(HEADER_START)).toBe(true);
        expect(text).toContain("https://zmre.github.io/ledgeline/invoicer/settings/");
        expect(text).toMatch(/^\[invoicer\]$/m);
        expect(text).toMatch(/^dso-days = 60$/m);
        // Only what changed: no other setting is written.
        const keys = text.split("\n").filter((line) => /^[a-z-]+ = /.test(line));
        expect(keys).toEqual(["dso-days = 60"]);

        // The page now reads the file: no "no settings file" note, and the setting is the file's.
        await expect(page.getByTestId("settings-no-file")).toHaveCount(0);
        await expect(page.getByTestId("setting-dso-days-use-default")).toBeVisible();
        // A second save writes to it without asking again.
        await page.getByTestId("input-paid-window-days").fill("45");
        await page.keyboard.press("ControlOrMeta+s");
        await expect(page.getByTestId("settings-saved")).toHaveText("Saved.");
        await expect(ask).toHaveCount(0);
        expect(readFileSync(SETTINGS_FILE, "utf8")).toMatch(/^paid-window-days = 45$/m);
    });
});
