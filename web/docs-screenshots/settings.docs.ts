// docs/settings/ — the account list editor and Account Aliases. The aliases
// shown are the three stage.ts appends to the staged copy of sample.journal.
import {expect, shot, test} from "./harness";

const SECTION = "settings";

test("accounts", async ({page}) => {
    await page.goto("/settings");
    await page.getByText("assets:bank:checking", {exact: true}).click();
    const editor = page.getByTestId("settings-account-editor");
    await expect(editor).toBeVisible();
    await expect(page.getByLabel("Type", {exact: true})).toHaveValue("C");
    await shot(page, SECTION, "accounts");
});

test("account aliases", async ({page}) => {
    await page.goto("/settings");
    await page.getByRole("tab", {name: "Account Aliases"}).click();
    const aliases = page.getByTestId("imports-aliases");
    await expect(aliases).toBeVisible();
    await expect(aliases.getByRole("textbox", {name: "What the bank calls it"}).first()).toHaveValue("Chase Checking 1234");
    await shot(page, SECTION, "account-aliases");
});
