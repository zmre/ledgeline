// Time tracking's Track screen end to end (plan 46 Phase 3): a digit starts
// a slot and the lamp lights; another digit switches (one lamp lit); `+`
// moves the running start back a step; `.` stops and banks the minutes; `n`
// writes a note, warning about a word hledger reads as a tag; the lamp on
// another page resumes with ⌥⌘digit (Ctrl+Shift+digit off a Mac) and stops
// with ⌥⌘. (Ctrl+Shift+.), the window title showing the timer meanwhile;
// the lamp's tracker panel (`T`, or a click) starts, moves and notes a slot
// from another page, wide and in the narrow top bar.
//
// Its own server: `--server` on :5194 over a scratch copy of
// invoicer/fixtures/time (target/e2e-invoicer-time/, staged fresh per run;
// playwright.config.ts). `vite preview` proxies only to :5199, so this spec
// loads the SPA the server embeds, as cash.e2e.ts does, and skips loudly
// when the binary embeds build.rs's placeholder (REQUIRE_REAL_SPA=1 makes
// that a failure).
//
// The server tracks on its own (real) clock, so nothing here depends on the
// fixture's date: the tracker writes this month's file,
// time/pat/YYYY-MM.timedot, which the roll-up's glob already includes. The
// person and the pins are set first through ui-prefs (kept in the server's
// memory: LEDGELINE_UI_PREFS_IN_BROWSER), so the slots are known:
//   1 Acme Corp · tax return, 2 Acme Corp · advisory, 3 Globex · audit,
//   4 Internal · admin.
//
// Serial: every step writes the copy's time file. Written with the feature
// and NOT run by the agent that wrote it (agents never run Playwright); the
// lead runs it.

import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import {expect, test, type Page} from "@playwright/test";
import {API_TOKEN, TIME_COPY, TIME_SERVER_URL} from "../playwright.config";

const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const BOOKS = resolve(import.meta.dirname, "..", TIME_COPY);
const PINS = ["clients:acme:tax-return", "clients:acme:advisory", "clients:globex:audit", "internal:admin"];

/** This month's tracker file in the copy, on the local clock (the server's). */
function trackFile(): string {
    const now = new Date();
    return `time/pat/${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}.timedot`;
}

const read = (file: string): string => {
    try {
        return readFileSync(resolve(BOOKS, file), "utf8");
    } catch {
        return "";
    }
};

/** The app-wide chord for `key` on this browser's platform. */
async function chord(page: Page, key: string): Promise<string> {
    const mac = await page.evaluate(() => /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent));
    return mac ? `Meta+Alt+${key}` : `Control+Shift+${key}`;
}

test.use({baseURL: TIME_SERVER_URL});
test.describe.configure({mode: "serial"});

/** Open `path` in the SPA the server embeds, with the launch token; skip when it is the placeholder. */
async function open(page: Page, path: string): Promise<void> {
    const shell = await page.request.get(`${TIME_SERVER_URL}/app/`);
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why = "The server embeds build.rs's PLACEHOLDER. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer time-track.e2e: ${why}\n`);
        test.skip(true, why);
    }
    await page.goto(`${TIME_SERVER_URL}/app${path}#token=${API_TOKEN}`);
    await expect(page).toHaveURL(new RegExp(`/app${path}$`));
}

test.beforeAll(async ({request}) => {
    const saved = await request.patch(`${TIME_SERVER_URL}/api/ui-prefs`, {
        headers: {Authorization: `Bearer ${API_TOKEN}`},
        data: {time: {person: "pat", pinned: PINS}},
    });
    expect(saved.ok()).toBe(true);
});

test("a digit starts a slot and the lamp lights; another switches", async ({page}) => {
    await open(page, "/time/track");
    const slots = page.getByTestId("time-slots");
    await expect(slots.getByRole("button")).toHaveCount(10);
    await expect(page.getByTestId("time-slot-2")).toContainText("Acme Corp · advisory");

    await page.keyboard.press("2");
    await expect(page.getByTestId("time-slot-2")).toHaveAttribute("aria-pressed", "true");
    const lamp = page.getByTestId("time-lamp-running");
    await expect(lamp).toHaveAttribute("aria-label", /^Time tracking running: Acme Corp · advisory/);
    await expect.poll(() => read(trackFile())).toContain("running clients:acme:advisory");

    await page.keyboard.press("3");
    await expect(page.getByTestId("time-slot-3")).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("time-slot-2")).toHaveAttribute("aria-pressed", "false");
    await expect(lamp).toHaveAttribute("aria-label", /^Time tracking running: Globex · audit/);
    await expect.poll(() => read(trackFile())).toContain("running clients:globex:audit");
    await expect.poll(() => read(trackFile())).not.toContain("running clients:acme:advisory");
});

test("+ moves the running start back a step; . stops and banks it", async ({page}) => {
    await open(page, "/time/track");
    await expect(page.getByTestId("time-slot-3")).toHaveAttribute("aria-pressed", "true");
    await page.getByTestId("time-slot-3").focus();
    await page.keyboard.type("+");
    await expect(page.getByTestId("time-lamp-elapsed")).toHaveText(/^[56]m \d\ds$/);
    await page.keyboard.press(".");
    await expect(page.getByTestId("time-lamp-stopped")).toHaveAttribute("aria-label", "Time tracking stopped, last Globex · audit");
    await expect.poll(() => read(trackFile())).toMatch(/clients:globex:audit\s+[56]m/);
    await expect.poll(() => read(trackFile())).not.toContain("running clients:globex:audit");
});

test("n writes a note, warning about a word hledger would read as a tag", async ({page}) => {
    await open(page, "/time/track");
    await page.getByTestId("time-slot-3").focus();
    await page.keyboard.press("n");
    const input = page.getByTestId("time-note-input");
    await expect(input).toBeFocused();
    await input.fill("kickoff re: scope");
    await expect(page.getByTestId("time-note-problem")).toContainText("“re:” would be read as a tag");
    await expect(page.getByTestId("time-note-save")).toBeDisabled();
    await page.getByTestId("time-note-suggestion").click();
    await expect(input).toHaveValue("kickoff re scope");
    await input.press("Enter");
    await expect(page.getByTestId("time-note")).toHaveCount(0);
    await expect.poll(() => read(trackFile())).toMatch(/clients:globex:audit\s+[56]m\s+; kickoff re scope/);
    await expect(page.getByTestId("time-slot-3")).toContainText("kickoff re scope");
});

test("the lamp on another page: the app-wide chord resumes and stops, the title shows the timer", async ({page}) => {
    await open(page, "/invoices");
    await expect(page.getByTestId("time-lamp-stopped")).toBeVisible();
    await expect(page.getByTestId("time-lamp-hint")).toHaveText(/^(⌥⌘3|Ctrl\+Shift\+3) to resume$/);

    await page.keyboard.press(await chord(page, "Digit3"));
    await expect(page.getByTestId("time-lamp-running")).toHaveAttribute("aria-label", /^Time tracking running: Globex · audit/);
    await expect(page).toHaveTitle(/^▶ \d+s · /);
    await expect(page).toHaveURL(/\/app\/invoices$/);

    // Switch from here: slot 4.
    await page.keyboard.press(await chord(page, "Digit4"));
    await expect(page.getByTestId("time-lamp-running")).toHaveAttribute("aria-label", /^Time tracking running: Internal · admin/);

    await page.keyboard.press(await chord(page, "Period"));
    await expect(page.getByTestId("time-lamp-stopped")).toHaveAttribute("aria-label", "Time tracking stopped, last Internal · admin");
    await expect(page).not.toHaveTitle(/^▶/);

    await expect.poll(() => read(trackFile())).not.toContain("running ");
});

test("the lamp is the tracker: T opens it, a digit starts, + moves, n notes, Esc closes back to the lamp", async ({page}) => {
    await open(page, "/invoices");
    const lamp = page.getByTestId("time-lamp-stopped");
    await expect(lamp).toHaveAttribute("aria-expanded", "false");

    await page.keyboard.press("Shift+T");
    const panel = page.getByRole("dialog", {name: "Time tracker"});
    await expect(panel).toBeVisible();
    await expect(page.getByTestId("time-lamp-slots").getByRole("button")).toHaveCount(10);
    await expect(page.getByTestId("time-lamp-slot-2")).toContainText("Acme Corp · advisory");

    await page.keyboard.press("2");
    await expect(page.getByTestId("time-lamp-slot-2")).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("time-lamp-running")).toHaveAttribute("aria-label", /^Time tracking running: Acme Corp · advisory/);
    await expect.poll(() => read(trackFile())).toContain("running clients:acme:advisory");
    await expect(page).toHaveURL(/\/app\/invoices$/);

    await page.keyboard.type("+");
    await expect(page.getByTestId("time-lamp-elapsed")).toHaveText(/^[56]m \d\ds$/);

    await page.keyboard.press("n");
    const input = panel.getByTestId("time-note-input");
    await expect(input).toBeFocused();
    await input.fill("panel note");
    await input.press("Enter");
    await expect(panel.getByTestId("time-note")).toHaveCount(0);
    await expect(panel).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
    await expect(page.getByTestId("time-lamp-running")).toBeFocused();

    // A click opens it too; Stop stops; Open Track goes to the full page.
    await page.getByTestId("time-lamp-running").click();
    await page.getByTestId("time-lamp-stop").click();
    await expect(page.getByTestId("time-lamp-stopped")).toBeVisible();
    // The note sits on the account's line today (a running timer's goes on its open line).
    await expect.poll(() => read(trackFile())).toMatch(/clients:acme:advisory\s+\S+\s+; panel note/);
    await expect.poll(() => read(trackFile())).not.toContain("running ");
    await page.getByTestId("time-lamp-open-track").click();
    await expect(page).toHaveURL(/\/app\/time\/track$/);
    await expect(panel).toHaveCount(0);
});

test("the lamp's tracker in the narrow top bar", async ({page}) => {
    await page.setViewportSize({width: 420, height: 800});
    await open(page, "/invoices");
    await page.getByTestId("time-lamp-stopped").click();
    const panel = page.getByRole("dialog", {name: "Time tracker"});
    await expect(panel).toBeVisible();
    // Under the bar, inside the window.
    const box = await panel.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(420);
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
});
