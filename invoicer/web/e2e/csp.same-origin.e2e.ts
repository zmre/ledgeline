// The browser oracle for the invoicer's two policies (PLAN A3), as
// web/e2e/csp.same-origin.e2e.ts is for Ledgeline's: the SPA the SERVER
// embeds, loaded from :5199 itself under the shell's script-hash CSP, framing
// MBR's page for an invoice under MBR's own CSP. Every other spec loads the SPA
// from `vite preview`, which sends no CSP at all, so only this one can see a
// policy refuse the shell's scripts or the frame.
//
// The server must embed the REAL SPA: `just invoicer-e2e` builds invoicer/web
// before the binary. A binary built first serves build.rs's placeholder, and
// this spec skips, loudly (REQUIRE_REAL_SPA=1 makes that a failure, for CI).
//
// Written in 3b and not run by the agent that wrote it; the lead runs it.

import {expect, test} from "@playwright/test";
import {API_TOKEN, SERVER_URL} from "../playwright.config";

const FIXED_NOW = new Date(2026, 10, 17, 12, 0, 0);
const PLACEHOLDER_MARKER = "Ledgeline Invoicer SPA not built";
const CSP_CONSOLE = /Content Security Policy|Refused to/i;

test.use({baseURL: SERVER_URL});

test("the embedded SPA and an invoice's frame load with no CSP violations", async ({page, request}) => {
    const shell = await request.get("/app/");
    expect(shell.status()).toBe(200);
    const csp = shell.headers()["content-security-policy"] ?? "";
    expect(csp.split(";").find((d) => d.trim().startsWith("script-src"))).not.toContain("unsafe-inline");
    const body = await shell.text();
    if (body.includes(PLACEHOLDER_MARKER) || !body.includes("__sveltekit")) {
        const why =
            "The server on :5199 embeds build.rs's PLACEHOLDER, not the SvelteKit build. Run `just invoicer-e2e` (it builds invoicer/web before the binary).";
        if (process.env.REQUIRE_REAL_SPA === "1") throw new Error(`REQUIRE_REAL_SPA=1: ${why}`);
        console.warn(`\n*** SKIPPING invoicer csp.same-origin: ${why}\n`);
        test.skip(true, why);
    }

    const violations: string[] = [];
    page.on("console", (message) => {
        if (CSP_CONSOLE.test(message.text())) violations.push(`console.${message.type()}: ${message.text()}`);
    });
    page.on("pageerror", (error) => violations.push(`pageerror: ${error.message}`));
    await page.clock.setFixedTime(FIXED_NOW);

    await page.goto(`/app/#token=${API_TOKEN}`);
    await expect(page.getByTestId("invoice-row")).toHaveCount(8);
    await page.goto("/app/invoices/105");
    const invoice = page.frameLocator('[data-testid="invoice-frame"]');
    await expect(invoice.getByText("Bill to Customer X")).toBeVisible();
    await expect(invoice.getByText("liabilities:customer-deposits")).toBeHidden();

    // A preview frame: a page of its own under MBR's policy (never srcdoc,
    // which would inherit the shell's frame-ancestors 'none').
    await page.getByTestId("template-choice").selectOption("/invoices/tpl/modern.html");
    await expect(invoice.getByText("Billed to")).toBeVisible();
    await expect(invoice.getByText("Less deposit (Invoice 104)")).toBeVisible();

    expect(violations).toEqual([]);
});
