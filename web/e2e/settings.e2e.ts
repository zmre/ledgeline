// The Settings tab: the Account List Editor's home, plus Account Aliases
// (moved here from Imports).
//
// # Why nothing here writes
//
// Same reasoning `budget.e2e.ts` gives for its own screen: the e2e engine is
// launched over `fixtures/sample.journal` with editing enabled, and that
// journal's 36 `account` declarations are what five other specs (balance
// sheet, holdings, …) assert exact numbers against. A spec that declared or
// retagged an account would leave the working tree dirty and stop being
// idempotent the second time it ran. The write path is covered instead by
// `crates/ledgeline-server/tests/account_endpoints.rs`, asserting the written
// BYTES — a stronger check than clicking Save and reading a badge. What this
// spec is for is the half that only exists in the browser: the tab is
// reachable, an existing declaration's type/tags/note show correctly, and the
// old `?tab=aliases` bookmark still works.
import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const API_URL = "http://127.0.0.1:5099";

test.beforeEach(async ({page}) => {
    await page.addInitScript(
        ([url, token]) => {
            localStorage.setItem("ledgeline.settings.v1", JSON.stringify({serverUrl: url, serverToken: token}));
        },
        [API_URL, API_TOKEN]
    );
});

test("settings: is reachable from the gear icon and lists a declared account", async ({page}) => {
    await page.goto("/");
    await page.getByRole("link", {name: "Settings"}).click();
    await expect(page).toHaveTitle("Ledgeline — Settings");

    await page.getByTestId("settings-accounts-filter").fill("assets:bank:checking");
    await page.getByText("assets:bank:checking", {exact: true}).click();

    const editor = page.getByTestId("settings-account-editor");
    await expect(editor).toBeVisible();
    await expect(editor).toContainText("assets:bank:checking");
    // `fixtures/sample.journal` declares this account `; type: C`. `exact`,
    // for the reason `assets:bank:checking` above needs it: the field's own
    // help button is labelled "About Type", a substring match away from
    // colliding with the field itself.
    await expect(page.getByLabel("Type", {exact: true})).toHaveValue("C");
});

test("settings: a bookmarked /imports?tab=aliases URL is forwarded", async ({page}) => {
    // Account Aliases used to be a tab under Imports and is under Settings now.
    await page.goto("/imports?tab=aliases");
    await expect(page).toHaveURL(/\/settings\?tab=aliases/);
    await expect(page.getByTestId("imports-aliases")).toBeVisible();

    // …and it is gone from the Imports tab strip, which is the other half of the move.
    await page.goto("/imports");
    await expect(page.getByRole("tab", {name: "Account Aliases"})).toHaveCount(0);
});
