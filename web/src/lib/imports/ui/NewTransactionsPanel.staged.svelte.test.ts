// The New Transactions screen WITH A FILE STAGED and nothing else requested —
// the other three of the five things that shipped broken on this screen.
//
// `NewTransactionsPanel.svelte.test.ts` covers the at-rest half (the drop zone
// claiming to be reading a file nobody dropped). This covers what the user sees
// one step later: with a converted file on screen and no dry run and no write
// asked for, the destination and balance fields were DISABLED and the action
// button wore a spinner. Same single mistake — `formBusy` was
// `dryRunView === "loading" || committedView === "loading"` written inside the
// component, and `dataView` reports a store that has never been asked for
// anything as "loading" — so the form froze before the user had done anything at
// all. There was nowhere to put a test on it: a condition written in a `.svelte`
// file could not be tested in this repo.
//
// It has since moved into `importStore.formBusy` and grown a unit test, which
// covers the expression. This covers the wiring: that the panel asks THAT
// question and not the old one, and that the answer reaches the `disabled`
// attribute of the fields it is about.
//
// # Why this is a second file
//
// `importStore` is a module singleton and staging a file mutates it. Vitest
// gives each test FILE its own module registry, which is the cheapest honest way
// to have both a pristine store and a staged one in the same suite.

import {importStore} from "$lib/imports/importStore.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {CAPABILITIES, STAGE, upload} from "$lib/testing/importFixtures";
import {render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import NewTransactionsPanel from "./NewTransactionsPanel.svelte";

/** The rendered control behind a testid, as something with a `disabled` flag. */
const control = (testid: string): HTMLInputElement | HTMLSelectElement | HTMLButtonElement =>
    screen.getByTestId(testid) as HTMLInputElement | HTMLSelectElement | HTMLButtonElement;

beforeEach(async () => {
    await connectFakeEngine({"/api/import/capabilities": CAPABILITIES, "/api/import/stage": STAGE});
    await importStore.reloadCapabilities(FAKE_ENGINE);
    // Staged BEFORE the render so the whole form is present on first paint,
    // which is the moment the frozen version of this screen was frozen at.
    await importStore.offerFile(upload("bank.csv"));
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT NewTransactionsPanel — a file is staged and nothing has been run", () => {
    it("shows the form the staged file unlocked", () => {
        render(NewTransactionsPanel);

        expect(screen.getByTestId("imports-destinations")).toBeDefined();
        expect(screen.getByTestId("imports-balance")).toBeDefined();
        expect(screen.getByTestId("imports-actions")).toBeDefined();
    });

    it("leaves the destination fields editable", () => {
        render(NewTransactionsPanel);

        expect(control("imports-csv-path").disabled).toBe(false);
        expect(control("imports-journal").disabled).toBe(false);
        // Seeded from the chosen candidate, not from the file's own default —
        // proof the form is genuinely wired and not merely present.
        expect((control("imports-csv-path") as HTMLInputElement).value).toBe("import/2026/bank.csv");
    });

    it("leaves the balance fields editable", () => {
        render(NewTransactionsPanel);

        expect(control("imports-balance-amount").disabled).toBe(false);
        expect((control("imports-balance-amount") as HTMLInputElement).value).toBe("-3238.65");
        // The statement volunteered a balance, so the assertion checkbox is on
        // screen too and is subject to the same freeze.
        expect(control("imports-write-assertion").disabled).toBe(false);
    });

    it("offers an action button that is pressable and not spinning", () => {
        const {container} = render(NewTransactionsPanel);

        expect(control("imports-submit").disabled).toBe(false);
        expect(container.querySelectorAll(".loading-spinner")).toHaveLength(0);
    });
});
