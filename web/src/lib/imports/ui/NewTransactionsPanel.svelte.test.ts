// The New Transactions screen AT REST — the state a user is in every time they
// open the tab, and the state that shipped broken.
//
// # The bug
//
// Five things were wrong on this screen at once, and they were all one mistake:
// a `DataView` was read as if it meant "busy". `dataView` has no idle branch —
// it reports a store that has never been asked for anything as `"loading"`,
// correctly, for the mount-fetching surfaces it was written for. This screen's
// resources sit idle until the user acts. So at rest the panel showed a spinner
// and "Reading the file…" in a drop zone nothing had been dropped on, the
// destination and balance fields were disabled, and the Save button wore a
// spinner nobody had triggered.
//
// Every unit test passed, because every unit was right. `stagingInFlight`,
// `formBusy` and `isInFlight` are all tested in `importStore.test.ts` and
// `importModel.test.ts` and all return exactly what they should. The component
// asked the wrong ones. Only a mount can catch that.
//
// # Why this file mounts nothing but the idle state
//
// `importStore` is a module singleton, so staging a file here would leak into
// every test below it and "at rest" would stop meaning at rest. The staged half
// of the same bug lives in `NewTransactionsPanel.staged.svelte.test.ts`, which
// vitest runs in its own module registry.

import {importStore} from "$lib/imports/importStore.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {CAPABILITIES} from "$lib/testing/importFixtures";
import {render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import NewTransactionsPanel from "./NewTransactionsPanel.svelte";

beforeEach(async () => {
    await connectFakeEngine({"/api/import/capabilities": CAPABILITIES});
    // The tab host probes capabilities, not this panel, so the test plays host.
    await importStore.reloadCapabilities(FAKE_ENGINE);
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT NewTransactionsPanel — before anything has been dropped", () => {
    it("invites a drop instead of claiming to be reading a file", () => {
        render(NewTransactionsPanel);

        expect(screen.getByTestId("imports-new")).toBeDefined();
        expect(screen.getByTestId("imports-drop-target")).toBeDefined();
        expect(screen.getByRole("heading", {name: "Drop a statement here"})).toBeDefined();
        expect(screen.getByRole("button", {name: "Choose file…"})).toBeDefined();
        // Both spellings of the wrong state: the spinner's accessible name and
        // the sentence beside it.
        expect(screen.queryByLabelText("Reading the file")).toBeNull();
        expect(screen.queryByText(/Reading the file/)).toBeNull();
    });

    it("shows no spinner anywhere", () => {
        // The whole class in one assertion. Four surfaces on this screen can
        // spin — the capabilities probe, the drop zone, the dry run and the
        // write — and at rest not one of them has been asked for anything.
        const {container} = render(NewTransactionsPanel);

        expect(container.querySelectorAll(".loading-spinner")).toHaveLength(0);
    });

    it("offers no form to freeze, because nothing is staged yet", () => {
        // The precondition the sibling file depends on. If a staged section ever
        // renders at rest, "the fields are enabled" over there stops describing
        // this screen's first paint.
        render(NewTransactionsPanel);

        expect(screen.queryByTestId("imports-destinations")).toBeNull();
        expect(screen.queryByTestId("imports-balance")).toBeNull();
        expect(screen.queryByTestId("imports-actions")).toBeNull();
    });
});
