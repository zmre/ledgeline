// Guards the ONE structural property that FE-5 was: on every surface backed by
// an async store, the error branch must be reachable.
//
// It was not. Each surface tested "render the data" first and then asked for an
// error AND `report === null` — a combination that cannot occur once anything
// has loaded. So a refetch that failed simply kept the previous answer on
// screen, relabelled by controls that had already moved: December's balance
// sheet under a June as-of, the balance sheet itself under the P&L tab (FE-1),
// `$0.00` and "no transactions match the current filters" for a journal that was
// never read (FE-5b). Nothing on screen said anything had failed.
//
// This reads the templates as text, which is unusual and worth justifying: the
// vitest config here has a single `node` project and explicitly EXCLUDES
// `*.svelte.test.ts`, so there is no component renderer to mount these in, and
// Chromium cannot launch in this environment either. The property is about
// branch ORDER in a template, so source order is the honest thing to assert.
// The markers are the `data-testid`s the e2e suite already selects on.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";

const WEB_SRC = fileURLToPath(new URL("..", import.meta.url));

const read = (path: string): string => readFileSync(`${WEB_SRC}/${path}`, "utf8");

/** Source with comments blanked out, so a comment ABOUT the old code doesn't read as the old code. */
const readCode = (path: string): string => read(path).replace(/<!--[\s\S]*?-->|\/\*[\s\S]*?\*\/|\/\/[^\n]*/g, "");

/** Each surface: its file, the marker for the error branch, and a marker only the data branch renders. */
const SURFACES = [
    {name: "reports route", file: "routes/reports/+page.svelte", error: 'data-testid="reports-error"', data: "<ReportTable"},
    {name: "holdings route", file: "routes/holdings/+page.svelte", error: 'data-testid="holdings-error"', data: 'data-testid="holdings-insights"'},
    {
        name: "insights dashboard",
        file: "lib/reports/ui/insights/InsightsDashboard.svelte",
        error: 'data-testid="insights-error"',
        data: 'testid="insights-box-revenue"',
    },
    {
        name: "subscriptions panel",
        file: "lib/reports/ui/subscriptions/SubscriptionsPanel.svelte",
        error: 'data-testid="subscriptions-error"',
        data: 'testid="subs-box-annual"',
    },
    {name: "journal route", file: "routes/+page.svelte", error: 'data-testid="journal-error"', data: "<TransactionTable"},
] as const;

describe("UNIT data surfaces keep the error branch reachable (FE-1 / FE-5)", () => {
    it.each(SURFACES)("$name renders its error branch before its data branch", ({file, error, data}) => {
        const source = read(file);
        const errorAt = source.indexOf(error);
        const dataAt = source.indexOf(data);
        expect(errorAt, `${file}: no ${error} branch`).toBeGreaterThanOrEqual(0);
        expect(dataAt, `${file}: no data branch`).toBeGreaterThanOrEqual(0);
        expect(errorAt, `${file}: the data branch is tested first, so the error branch is dead once anything has loaded`).toBeLessThan(dataAt);
    });

    it.each(SURFACES)("$name does not gate its error branch on having no payload", ({file}) => {
        // `report === null && status === "error"` — the exact conjunction that
        // made every one of these unreachable.
        const offenders = [...readCode(file).matchAll(/\w+ === null && \w+(\.\w+)*\.status === "error"/g)].map((m) => m[0]);
        expect(offenders).toEqual([]);
    });

    it.each([
        {file: "routes/reports/+page.svelte", gate: "stylesReady"},
        {file: "routes/holdings/+page.svelte", gate: "journal.txns.length > 0"},
    ])("$file does not gate readiness on the journal feed's row count (FE-5c)", ({file, gate}) => {
        // Both pages are driven by the ENGINE's /api/* report but waited on a row
        // count from the separate hledger-web feed. A working engine plus a
        // failing feed spun forever — and so did a legitimately empty journal, on
        // every new user's first run.
        expect(readCode(file)).not.toContain(gate);
    });
});
