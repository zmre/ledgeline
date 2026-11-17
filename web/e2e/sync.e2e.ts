// Bank sync (plan 43) end to end, against the sync engine on :5093: a scratch
// copy of sample.journal (target/e2e-sync/, staged fresh per run) whose
// SimpleFIN is the fake bridge in fixtures/feeds/fake-bridge/ (see its README
// for what each account shows). Linking writes files beside the journal, which
// is why this engine has a copy of its own.
//
// CI runs the Nix release binary, which has no test fakes and ignores
// LEDGELINE_FEED_FAKE, so the spec skips there ($LEDGELINE_BIN is set).

import {readFileSync} from "node:fs";
import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const SYNC_API = "http://127.0.0.1:5093";
const COPY = new URL("../../target/e2e-sync/", import.meta.url);
/** `https://fake.simplefin.test/claim/DEMO`, a token the fake bridge accepts. */
const FAKE_TOKEN = "aHR0cHM6Ly9mYWtlLnNpbXBsZWZpbi50ZXN0L2NsYWltL0RFTU8=";

test.skip(process.env.LEDGELINE_BIN !== undefined, "CI's release binary has no fake bridge (needs --features test-fakes)");

// One linked state carries through: connect → link → sync → disconnect.
test.describe.configure({mode: "serial"});

test.beforeEach(async ({page}) => {
    await page.addInitScript(
        ([url, token]) => localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token})),
        [SYNC_API, API_TOKEN]
    );
});

test("connects SimpleFIN, links two accounts and runs the first sync", async ({page}) => {
    await page.goto("/imports?tab=accounts");
    await page.getByTestId("sync-connect").click();
    const wizard = page.getByTestId("sync-wizard");
    await expect(wizard.getByRole("heading", {name: "How should Ledgeline get your transactions?"})).toBeVisible();
    await wizard.getByRole("button", {name: "Next"}).click();

    await wizard.getByTestId("sync-token").fill(FAKE_TOKEN);
    await expect(wizard.getByTestId("sync-token-check")).toBeVisible();
    await wizard.getByRole("button", {name: "Connect", exact: true}).click();
    await expect(wizard.getByRole("heading", {name: /SimpleFIN found 3 accounts/})).toBeVisible();

    // Lakeside and Harbor are suggested onto the journal's accounts; Summit
    // has no match yet, so it isn't synced this time.
    const summit = wizard.locator('[data-testid="sync-link-form"][data-remote="ACT-summit-savings"]');
    await summit.getByLabel("Don't sync").check();

    await wizard.getByTestId("sync-preview").click();
    await expect(wizard.getByTestId("sync-preview-row")).toHaveCount(2);
    await wizard.getByTestId("sync-finish").click();
    await expect(wizard.getByRole("heading", {name: "Linked"})).toBeVisible();
    await expect(wizard.getByTestId("sync-done-summary")).toContainText("6");
    await wizard.getByRole("button", {name: "Done"}).click();

    await expect(page.getByTestId("sync-linked-row")).toHaveCount(2);

    // What landed beside the journal: ids on every synced row, Trader Joe's
    // transacted date as hledger's secondary date, and the links (never a
    // credential) in ledgeline.toml.
    const journal = readFileSync(new URL("sample.journal", COPY), "utf8");
    expect(journal).toContain("2026-07-06 Acme Corp  ; id:");
    expect(journal).toContain("2026-07-10=2026-07-09 Trader Joe's");
    expect(journal).not.toContain("SAFEWAY"); // held: it matches the journal's 07-03 Safeway
    const toml = readFileSync(new URL("ledgeline.toml", COPY), "utf8");
    expect(toml).toContain('remoteId = "ACT-lakeside-checking"');
    expect(toml).not.toMatch(/https?:\/\//);
});

test("Sync now adds nothing new the second time", async ({page}) => {
    await page.goto("/imports?tab=accounts");
    await expect(page.getByTestId("sync-linked-row")).toHaveCount(2);
    const before = readFileSync(new URL("sample.journal", COPY), "utf8");
    await page.getByTestId("sync-now").first().click();
    await expect(page.getByTestId("sync-toast")).toBeVisible();
    await expect(page.getByTestId("sync-toast")).not.toContainText("failed");
    expect(readFileSync(new URL("sample.journal", COPY), "utf8")).toBe(before);
});

test("disconnecting keeps the links, and Connect… reattaches the same connection", async ({page}) => {
    await page.goto("/settings?tab=sync");
    const row = page.getByTestId("sync-settings-connection");
    await row.getByRole("button", {name: "Disconnect…"}).click();
    await row.getByTestId("sync-disconnect").getByRole("button", {name: "Disconnect", exact: true}).click();

    // ledgeline.toml is shared with other computers: the links stay.
    const toml = readFileSync(new URL("ledgeline.toml", COPY), "utf8");
    expect(toml).toContain("[[sync.connection]]");
    expect(toml).toContain('remoteId = "ACT-harbor-visa"');

    await page.goto("/imports?tab=accounts");
    const problem = page.locator('[data-testid="sync-problem"][data-code="not-connected"]');
    await expect(problem.first()).toBeVisible();
    await problem.first().getByRole("button", {name: "Connect…"}).click();
    const wizard = page.getByTestId("sync-wizard");
    await expect(wizard.getByRole("heading", {name: "Paste a new setup token"})).toBeVisible();
    await wizard.getByTestId("sync-token").fill(FAKE_TOKEN);
    await wizard.getByRole("button", {name: "Connect", exact: true}).click();
    await expect(page.locator('[data-testid="sync-problem"][data-code="not-connected"]')).toHaveCount(0);
    await expect(page.getByTestId("sync-linked-row")).toHaveCount(2);
});
