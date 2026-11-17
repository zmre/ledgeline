// docs/imports/bank-sync.md — the Accounts tab, the connect wizard, the share
// dialog, a held transaction, the account drawer, Settings → Sync, and one
// problem. Engines: the `sync` set (fixtures/feeds/fake-bridge/) and the
// `syncBroken` set (fake-bridge-broken/, Harbor already linked), both staged
// copies under target/docs-screenshots/, so linking writes nowhere committed.
//
// The browser's clock is the real one here, not FIXED_NOW: the engine stamps
// syncs with its own wall clock, and a pinned browser would read them as
// months in the future. Relative times ("just now") stay the same either way.
import type {Locator, Page} from "@playwright/test";
import {around, expect, shot, test} from "./harness";

const SECTION = "imports";
/** `https://fake.simplefin.test/claim/DEMO`: the fake bridge accepts it. */
const FAKE_TOKEN = "aHR0cHM6Ly9mYWtlLnNpbXBsZWZpbi50ZXN0L2NsYWltL0RFTU8=";
/** `https://beta-bridge.simplefin.org/simplefin/claim/DOCS`: what a real token looks like, for the token shot only. */
const BRIDGE_TOKEN = Buffer.from("https://beta-bridge.simplefin.org/simplefin/claim/DOCS").toString("base64");

const box = (dialog: Locator): Locator => dialog.locator(".modal-box").first();

test.describe("bank sync", () => {
    test.use({journal: "sync", now: new Date()});
    test.describe.configure({mode: "serial"});

    test("connect wizard, link, preview, accounts tab", async ({page}) => {
        await page.setViewportSize({width: 1280, height: 1100});
        await page.goto("/imports?tab=accounts");
        await page.getByTestId("sync-connect").click();
        const wizard = page.getByTestId("sync-wizard");
        await expect(wizard.getByRole("heading", {name: "How should Ledgeline get your transactions?"})).toBeVisible();
        await shot(page, SECTION, "bank-sync-wizard-choose", {clip: box(wizard), padding: 0});

        await wizard.getByRole("button", {name: "Next"}).click();
        await wizard.getByTestId("sync-token").fill(BRIDGE_TOKEN);
        await expect(wizard.getByTestId("sync-token-check")).toBeVisible();
        await wizard.getByTestId("sync-token").blur();
        await shot(page, SECTION, "bank-sync-wizard-token", {clip: box(wizard), padding: 0});

        await wizard.getByTestId("sync-token").fill(FAKE_TOKEN);
        await wizard.getByRole("button", {name: "Connect", exact: true}).click();
        await expect(wizard.getByRole("heading", {name: /SimpleFIN found 3 accounts/})).toBeVisible();
        // Summit has no journal account yet: not synced this time, so the
        // Accounts tab shows it as found but not linked.
        await wizard.locator('[data-testid="sync-link-form"][data-remote="ACT-summit-savings"]').getByLabel("Don't sync").check();
        const lakeside = wizard.locator('[data-testid="sync-link-form"][data-remote="ACT-lakeside-checking"]');
        await expect(lakeside.getByTestId("sync-share-prompt")).toBeVisible();
        await shot(page, SECTION, "bank-sync-wizard-link", {clip: lakeside, padding: 12});

        await lakeside.getByRole("button", {name: "Share them"}).click();
        const share = page.getByTestId("sync-share-dialog");
        await expect(share.getByTestId("sync-share-kept").or(share.getByTestId("sync-share-identical")).first()).toBeVisible();
        await shot(page, SECTION, "bank-sync-share-dialog", {clip: box(share), padding: 0});
        await share.getByTestId("sync-share-confirm").click();
        await expect(share).toHaveCount(0);

        await wizard.getByTestId("sync-preview").click();
        await expect(wizard.getByTestId("sync-preview-row")).toHaveCount(2);
        await shot(page, SECTION, "bank-sync-wizard-preview", {clip: box(wizard), padding: 0});

        await wizard.getByTestId("sync-finish").click();
        await expect(wizard.getByRole("heading", {name: "Linked"})).toBeVisible();
        await wizard.getByRole("button", {name: "Done"}).click();
        await expect(page.getByTestId("sync-linked-row")).toHaveCount(2);
        await page.setViewportSize({width: 1280, height: 900});
        // The connection and the first two manual-download rows: the sample
        // journal has a dozen balance-sheet accounts, and the rest of the list
        // looks the same.
        const accounts = await around([page.getByTestId("sync-accounts")]);
        const secondManual = await around([page.getByTestId("sync-manual-row").nth(1)]);
        await shot(page, SECTION, "bank-sync-accounts-tab", {clip: {...accounts, height: secondManual.y + secondManual.height - accounts.y}, padding: 12});
    });

    test("held transaction and the account drawer", async ({page}) => {
        // Short enough that the drawer has no long gap above Stop syncing.
        await page.setViewportSize({width: 1280, height: 760});
        await page.goto("/imports?tab=accounts");
        const harbor = page.getByTestId("sync-linked-row").filter({hasText: "Harbor"});
        await harbor
            .getByRole("button", {name: /Harbor/})
            .first()
            .click();
        const drawer = page.getByTestId("sync-account-drawer");
        await expect(drawer.getByTestId("sync-held-row")).toHaveCount(1);
        await shot(page, SECTION, "bank-sync-held", {clip: drawer.getByTestId("sync-held"), padding: 8});
        await shot(page, SECTION, "bank-sync-account-drawer", {clip: drawer.locator(":scope > *").first(), padding: 0});
    });

    test("settings", async ({page}) => {
        await page.goto("/settings?tab=sync");
        const panel = page.getByTestId("settings-sync");
        await expect(panel.getByTestId("sync-settings-connection")).toHaveCount(1);
        await shot(page, SECTION, "bank-sync-settings", {clip: panel, padding: 12});
    });
});

test.describe("bank sync problem", () => {
    test.use({journal: "syncBroken", now: new Date()});

    /** Store a token for the staged connection and run one sync, through the API. */
    const syncOnce = async (page: Page): Promise<void> => {
        const api = page.context().request;
        const settings = await page.evaluate(
            () => JSON.parse(localStorage.getItem("ledgeline.settings.v1") ?? "{}") as {serverUrl: string; serverToken: string}
        );
        const headers = {Authorization: `Bearer ${settings.serverToken}`};
        const connected = await api.post(`${settings.serverUrl}/api/sync/connections/docs-bridge/token`, {headers, data: {setupToken: FAKE_TOKEN}});
        expect(connected.ok()).toBe(true);
        const ran = await api.post(`${settings.serverUrl}/api/sync/now`, {headers, data: {}});
        expect(ran.ok()).toBe(true);
    };

    test("con-auth on a linked account", async ({page}) => {
        await page.goto("/imports?tab=accounts");
        await syncOnce(page);
        await page.reload();
        const problem = page.locator('[data-testid="sync-problem"][data-code="con-auth"]').first();
        await expect(problem).toBeVisible();
        const row = page.getByTestId("sync-linked-row").filter({has: problem});
        await shot(page, SECTION, "bank-sync-problem-con-auth", {clip: (await row.count()) > 0 ? row : problem, padding: 8});
    });
});
