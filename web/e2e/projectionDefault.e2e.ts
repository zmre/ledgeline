// E2E for the default projection (plans/33-default-projection.md): the star in
// the scenario file bar, what opens on a page load versus a tab switch, and
// the bytes the star leaves on disk.
//
// Against the default-projection engine on :5095, a SCRATCH COPY of
// fixtures/projection-default/ (target/e2e-projection-default/, staged fresh
// on every run by playwright.config.ts): this spec stars, un-stars and saves,
// and a write must never touch a committed fixture. The engine is launched once
// per run, so the steps below are ONE test, in order — each starts from the
// disk the previous one left.
//
// # Fixture facts
//
// `main.journal` includes `budget.journal` (a plan of record: readable, never
// written, never the default). `projection-alpha.journal` and
// `projection-beta.journal` are Ledgeline scenario files, Alpha first by id.
// Beta's one rule is `(revenues:salary) $-4000.00`.

import {readFileSync, writeFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5095";
const FIXED_NOW = new Date(2026, 10, 9, 12, 0, 0); // local 2026-11-09
const COPY = resolve(import.meta.dirname, "../../target/e2e-projection-default");
const MARKER = "; default-scenario: true";

const ALPHA = "projection-alpha.journal";
const BETA = "projection-beta.journal";

const onDisk = (file: string): string => readFileSync(resolve(COPY, file), "utf8").replaceAll("\r\n", "\n");

/**
 * Point the SPA at this engine WITHOUT clobbering the rest of the stored
 * settings: `lastProjectionId` has to survive a reload for step 8, and an init
 * script runs on every navigation.
 */
async function connect(page: Page): Promise<void> {
    await page.clock.setFixedTime(FIXED_NOW);
    await page.addInitScript(
        ([url, token]) => {
            const key = "ledgeline.settings.v1";
            const previous = JSON.parse(localStorage.getItem(key) ?? "{}") as Record<string, unknown>;
            localStorage.setItem(key, JSON.stringify({...previous, serverUrl: url, serverToken: token, insightsOpen: false}));
        },
        [API_URL, API_TOKEN]
    );
}

const picker = (page: Page) => page.getByTestId("projection-file-select");
const star = (page: Page) => page.getByTestId("projection-default-toggle");

/** Pick a file from the scenario picker and wait for it to be the one on screen. */
async function pick(page: Page, id: string): Promise<void> {
    await picker(page).selectOption(id);
    await expect(picker(page)).toHaveValue(id);
    await expect(page.getByTestId("projection-entry").first()).toBeVisible();
}

test("projections: the default scenario — star, open, survive a tab switch, relaunch, conflict, un-star", async ({page, browser}) => {
    await connect(page);
    await page.goto("/projections");

    await test.step("1. a seeded scenario cannot be starred until it is saved", async () => {
        await expect(picker(page)).toHaveValue("");
        await expect(star(page)).toBeDisabled();
        await expect(star(page)).toHaveAttribute("title", /Save this scenario to a file first/);
    });

    await test.step("2. the included budget cannot be the default; Alpha can, and the marker is on disk", async () => {
        await pick(page, "budget.journal");
        await expect(star(page)).toBeDisabled();
        await expect(star(page)).toHaveAttribute("title", /Only a scenario file Ledgeline saved/);

        await pick(page, ALPHA);
        await expect(star(page)).toBeEnabled();
        await expect(star(page)).toHaveAttribute("aria-pressed", "false");
        await star(page).click();
        await expect(star(page)).toHaveAttribute("aria-pressed", "true");
        await expect(picker(page).locator(`option[value="${ALPHA}"]`)).toHaveText("Alpha (default)");
        expect(onDisk(ALPHA)).toContain(MARKER);
        expect(onDisk(BETA)).not.toContain(MARKER);
    });

    await test.step("3. a tab switch keeps what was open (D2)", async () => {
        await pick(page, BETA);
        await page.getByRole("link", {name: "Reports"}).click();
        await expect(page).toHaveTitle(/Reports/);
        await page.getByRole("link", {name: "Projections"}).click();
        await expect(picker(page)).toHaveValue(BETA);
    });

    await test.step("4. starring Beta WITH unsaved edits keeps them, and the save keeps both (D17)", async () => {
        const card = page.getByTestId("projection-entry").first();
        await card.getByTestId("entry-edit").click();
        const dialog = page.getByTestId("entry-dialog");
        const amount = dialog.getByLabel("Amount for revenues:salary");
        await amount.fill("-4500");
        await amount.blur();
        await dialog.getByTestId("entry-dialog-done").click();
        await expect(dialog).toHaveCount(0);
        await expect(page.getByTestId("projection-dirty")).toBeVisible();

        await star(page).click();
        await expect(star(page)).toHaveAttribute("aria-pressed", "true");
        await expect(page.getByTestId("projection-dirty")).toBeVisible();
        expect(onDisk(ALPHA)).not.toContain(MARKER);
        expect(onDisk(BETA)).toContain(MARKER);

        await page.getByTestId("projection-save").click();
        await expect(page.getByTestId("projection-dirty")).toHaveCount(0);
        await expect(page.getByTestId("projection-file-error")).toHaveCount(0);
        const saved = onDisk(BETA);
        expect(saved).toContain(MARKER);
        expect(saved).toContain("$-4500");
    });

    await test.step("5. a reload is a fresh mount, and opens the default", async () => {
        await page.reload();
        await expect(picker(page)).toHaveValue(BETA);
        await expect(star(page)).toHaveAttribute("aria-pressed", "true");
    });

    await test.step("6. a relaunch (fresh storage) opens the default too", async () => {
        const context = await browser.newContext();
        const fresh = await context.newPage();
        await connect(fresh);
        await fresh.goto("/projections");
        await expect(picker(fresh)).toHaveValue(BETA);
        await context.close();
    });

    await test.step("7. two marked files: a warning, the first by id opens, and a star repairs it (D6)", async () => {
        writeFileSync(resolve(COPY, ALPHA), onDisk(ALPHA).replace("; updated: 2026-11-01\n", `; updated: 2026-11-01\n${MARKER}\n`));
        await page.reload();
        await expect(page.getByTestId("projection-default-conflict")).toContainText("2 scenario files are marked as the default");
        await expect(picker(page)).toHaveValue(ALPHA);

        await pick(page, BETA);
        await star(page).click();
        await expect(page.getByTestId("projection-default-conflict")).toHaveCount(0);
        await expect(star(page)).toHaveAttribute("aria-pressed", "true");
        expect(onDisk(ALPHA)).not.toContain(MARKER);
        expect(onDisk(BETA)).toContain(MARKER);
    });

    await test.step("8. un-starred, a reload behaves as before: the last file opened", async () => {
        await star(page).click();
        await expect(star(page)).toHaveAttribute("aria-pressed", "false");
        expect(onDisk(BETA)).not.toContain(MARKER);
        await page.reload();
        await expect(picker(page)).toHaveValue(BETA);
        await expect(star(page)).toHaveAttribute("aria-pressed", "false");
    });
});
