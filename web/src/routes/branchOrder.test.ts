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
//
// WHAT CHANGED (DRY-6). The four hand-written copies of that chain are gone —
// they are one `<AsyncSection>` now. So the ordering assertion moved to the one
// file that still expresses an order, and the four surfaces are instead checked
// for the thing that can now go wrong: quietly growing a SECOND, hand-rolled
// chain beside the shared one. That is a stronger guarantee than before, where
// each surface could independently regress. The journal route is deliberately
// not converted (its failure mode is a full-page panel, not a section), so it
// keeps the original order assertion.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";

const WEB_SRC = fileURLToPath(new URL("..", import.meta.url));

const read = (path: string): string => readFileSync(`${WEB_SRC}/${path}`, "utf8");

/** Source with comments blanked out, so a comment ABOUT the old code doesn't read as the old code. */
const readCode = (path: string): string => read(path).replace(/<!--[\s\S]*?-->|\/\*[\s\S]*?\*\/|\/\/[^\n]*/g, "");

const ASYNC_SECTION = "lib/components/AsyncSection.svelte";

/** Each surface delegating to AsyncSection: its file, and the error testid it must hand over. */
const SURFACES = [
    {name: "reports route", file: "routes/reports/+page.svelte", testid: "reports-error"},
    {name: "holdings route", file: "routes/holdings/+page.svelte", testid: "holdings-error"},
    {name: "insights dashboard", file: "lib/reports/ui/insights/InsightsDashboard.svelte", testid: "insights-error"},
    {name: "subscriptions panel", file: "lib/reports/ui/subscriptions/SubscriptionsPanel.svelte", testid: "subscriptions-error"},
    // The rules editor moved out of `routes/imports/+page.svelte` when that
    // became a tab host (WP-11); it is registered HERE, at the file that now
    // owns the async surface, so the move cannot quietly lose the guarantee.
    {name: "edit rules panel", file: "lib/imports/ui/EditRulesPanel.svelte", testid: "imports-error"},
    // The New Transactions flow (WP-11 lane E) is four async surfaces, not one,
    // and every one of them can hold a payload that a later request supersedes —
    // which is exactly the FE-1/FE-5 pair this file exists for. The dry run and
    // the commit are the dangerous ones: neither payload carries any field
    // naming the file, rules file or destination it was computed for, so a stale
    // one CANNOT be spotted by its own shape (see `sameRunRequest`).
    {name: "new transactions capabilities", file: "lib/imports/ui/NewTransactionsPanel.svelte", testid: "imports-capabilities-error"},
    {name: "staged file panel", file: "lib/imports/ui/StagedPanel.svelte", testid: "imports-stage-error"},
    {name: "dry run panel", file: "lib/imports/ui/DryRunPanel.svelte", testid: "imports-dry-run-error"},
    {name: "import result panel", file: "lib/imports/ui/ResultPanel.svelte", testid: "imports-commit-error"},
] as const;

describe("UNIT data surfaces keep the error branch reachable (FE-1 / FE-5)", () => {
    it("AsyncSection renders its error branch before its data branch", () => {
        // The whole property, in one place. `{#if view === "error"}` must come
        // before the branch that renders the payload, or the error branch is
        // dead once anything has loaded.
        const source = read(ASYNC_SECTION);
        const errorAt = source.indexOf('{#if view === "error"}');
        const dataAt = source.indexOf("{@render children(");
        expect(errorAt, `${ASYNC_SECTION}: no error branch`).toBeGreaterThanOrEqual(0);
        expect(dataAt, `${ASYNC_SECTION}: no data branch`).toBeGreaterThanOrEqual(0);
        expect(errorAt, `${ASYNC_SECTION}: the data branch is tested first, so the error branch is dead once anything has loaded`).toBeLessThan(dataAt);
    });

    it("AsyncSection does not gate its error branch on having no payload", () => {
        // `value === null && status === "error"` — the exact conjunction that
        // made every one of the old hand-written chains unreachable.
        expect(readCode(ASYNC_SECTION)).not.toMatch(/\w+ === null && \w+(\.\w+)*\.status === "error"/);
        // The error branch must ask about `view` ALONE. Anding anything onto it
        // is how the original defect was written.
        expect(readCode(ASYNC_SECTION)).toContain('{#if view === "error"}');
    });

    it.each(SURFACES)("$name delegates its tri-state to AsyncSection", ({file, testid}) => {
        const code = readCode(file);
        expect(code, `${file}: does not render <AsyncSection>`).toContain("<AsyncSection");
        expect(code, `${file}: does not hand AsyncSection its error testid`).toContain(`testid="${testid}"`);
    });

    it.each(SURFACES)("$name does not hand-roll a second branch chain beside the shared one", ({file, testid}) => {
        const code = readCode(file);
        // A literal `data-testid="…-error"` means an alert div was written here
        // rather than handed to AsyncSection as a prop — i.e. a private copy of
        // the chain has grown back.
        expect(code, `${file}: hand-rolled error alert instead of using AsyncSection`).not.toContain(`data-testid="${testid}"`);
        // And no local re-derivation of the branch order.
        expect(code, `${file}: re-tests the error branch locally`).not.toMatch(/view === "error"/);
    });

    it("the journal route, which is not an AsyncSection, still orders error before data", () => {
        // Its failure mode is a whole-page panel replacing the table, not a
        // section inside one, so it keeps its own chain — and its own guard.
        const file = "routes/+page.svelte";
        const source = read(file);
        const errorAt = source.indexOf('data-testid="journal-error"');
        const dataAt = source.indexOf("<TransactionTable");
        expect(errorAt, `${file}: no error branch`).toBeGreaterThanOrEqual(0);
        expect(dataAt, `${file}: no data branch`).toBeGreaterThanOrEqual(0);
        expect(errorAt, `${file}: the data branch is tested first, so the error branch is dead once anything has loaded`).toBeLessThan(dataAt);
        expect(readCode(file)).not.toMatch(/\w+ === null && \w+(\.\w+)*\.status === "error"/);
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
