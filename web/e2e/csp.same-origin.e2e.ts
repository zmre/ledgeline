// The browser oracle for the shell's Content-Security-Policy.
//
// Every other spec loads the SPA from `bun run preview` on :4173, CROSS-ORIGIN
// to the engine and with no CSP at all, so none of them can notice a policy that
// refuses the shell's own scripts. This one points the browser at the ENGINE
// (:5099) itself, which serves its embedded copy of the real SvelteKit build
// same-origin under the CSP that `security::shell_csp` computes from the served
// bytes: exactly what the packaged app's webview loads. If a SvelteKit upgrade
// ever emits a script the policy does not hash, Chromium refuses it, the app
// stays a blank page, and this fails.
//
// The engine has to be embedding the REAL SPA for that to mean anything:
//   - CI runs `nix build .#ledgelineWithSpa` and sets REQUIRE_REAL_SPA=1, so a
//     placeholder there is a FAILURE.
//   - Locally `just e2e` builds the SPA before the debug binary (which serves
//     whatever build.rs mirrored into crates/ledgeline-server/spa/). Run
//     Playwright some other way on a fresh checkout and this SKIPS, loudly.
//
// The token rides in the URL fragment (`/#token=…`), the headless `--server`
// handoff; never in a request, and nothing here asserts it is in the body.

import {expect, test} from "@playwright/test";
import {API_TOKEN} from "../playwright.config";

const ENGINE_URL = "http://127.0.0.1:5099";
const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0); // local 2026-07-08, as the sibling specs pin it

/** Only a SvelteKit shell has this: its bootstrap assigns `__sveltekit_<hash>`. */
const SVELTEKIT_MARKER = "__sveltekit";
/** build.rs's (and flake.nix's) placeholder heading. */
const PLACEHOLDER_MARKER = "Ledgeline SPA not built";

const CSP_CONSOLE = /Content Security Policy|Refused to/i;

test.use({baseURL: ENGINE_URL});

test("the engine-served SPA boots under its own CSP with no violations", async ({page, request}) => {
    const shell = await request.get("/");
    expect(shell.status()).toBe(200);
    const csp = shell.headers()["content-security-policy"] ?? "";
    expect(csp, "the shell must carry a script-src policy, or this test proves nothing").toContain("script-src");
    // style-src carries 'unsafe-inline'; script-src must never grow it as the "fix".
    expect(csp.split(";").find((directive) => directive.trim().startsWith("script-src"))).not.toContain("unsafe-inline");

    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes(SVELTEKIT_MARKER)) {
        const why =
            "The engine on :5099 is serving build.rs's PLACEHOLDER page, not the SvelteKit build, so the " +
            "same-origin CSP check cannot run. Locally: `just e2e` (it builds web/ before the binary). " +
            "CI: LEDGELINE_BIN must be a real-SPA binary (`nix build .#ledgelineWithSpa`).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING csp.same-origin: ${why}\n`);
        test.skip(true, why);
    }

    const violations: string[] = [];
    page.on("console", (message) => {
        if (CSP_CONSOLE.test(message.text())) violations.push(`console.${message.type()}: ${message.text()}`);
    });
    page.on("pageerror", (error) => violations.push(`pageerror: ${error.message}`));
    // The DOM's own report, independent of console wording. Init scripts are
    // injected by the browser, not the page, so the policy under test does not
    // apply to this one.
    await page.addInitScript(() => {
        const sink: string[] = [];
        (window as unknown as {__cspViolations: string[]}).__cspViolations = sink;
        document.addEventListener("securitypolicyviolation", (event) => {
            sink.push(`securitypolicyviolation: ${event.effectiveDirective} blocked ${event.blockedURI || "inline"}`);
        });
    });
    await page.clock.setFixedTime(FIXED_NOW);

    const allViolations = async (): Promise<string[]> => [
        ...violations,
        ...(await page.evaluate(() => (window as unknown as {__cspViolations: string[]}).__cspViolations)),
    ];

    await page.goto(`/#token=${API_TOKEN}`);
    // Checked BEFORE the landmarks so a refused script fails with the browser's
    // own reason rather than as a title timeout on a blank page.
    await page.waitForLoadState("networkidle");
    expect(await allViolations()).toEqual([]);

    // Landmarks that need the bootstrap script, the lazy route chunks AND an
    // authenticated same-origin API call to have all worked.
    await expect(page).toHaveTitle("Ledgeline — Journal");
    await expect(page.locator("footer")).toContainText("transactions");
    await expect(page.getByRole("heading", {name: "Connect to hledger-web"})).toHaveCount(0);

    // A client-side navigation pulls in another route's chunks under the same policy.
    await page.getByRole("link", {name: "Reports"}).click();
    await expect(page).toHaveTitle("Ledgeline — Reports");
    await expect(page.getByRole("tab", {name: "Balance Sheet"})).toBeVisible();

    expect(await allViolations()).toEqual([]);
});
