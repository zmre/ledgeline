// Mounting the Account Aliases tab — which is the only way to catch the defect
// this panel shipped.
//
// # The bug
//
// The panel seeds its form in an `$effect` latched on the chosen file. The latch
// was written as `let baseFile = $state<AliasFile | null>(null)` and compared
// with `===` against the raw object from the listing, inside the same effect
// that assigns it. `$state` deep-proxies objects on assignment, so `baseFile`
// holds a PROXY and the comparison is never true; `$state` is also tracked, so
// the effect reads a signal it writes and depends on itself. Svelte spins it
// until `effect_update_depth_exceeded` throws — and that error does not merely
// break this panel, it kills the whole app: every nav link and every button
// stops responding, with nothing on screen saying why.
//
// Every unit test passed. The bug is not in a function; it is in what happens
// when the component is mounted, so mounting it is the test.
//
// `effectLatch.test.ts` lints the same defect out of the source text across
// every file at once, which is cheaper and broader. This is the other half: it
// proves the panel actually survives a mount, which a text lint cannot claim,
// and it fails on any FUTURE way of writing a self-feeding effect rather than
// only on the one shape that regex knows.
//
// # Why the store is real
//
// The listing arrives through a stubbed `fetch` rather than a mocked
// `aliasStore`, because a mocked store would prove the panel renders what it is
// handed — and the panel renders what it is handed perfectly well. What broke
// was what it did with it.

import {aliasStore} from "$lib/imports/aliasStore.svelte";
import {settings} from "$lib/stores/settings.svelte";
import {connectFakeEngine, FAKE_ENGINE} from "$lib/testing/fakeEngine";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import AliasPanel from "./AliasPanel.svelte";

/** `GET /api/aliases` as the engine sends it. Two rows so "kept what was there" is countable. */
const ALIASES = {
    editable: true,
    files: [
        {
            journalId: "2026/2026.journal",
            label: "2026.journal",
            revision: "rev-1",
            writable: true,
            aliases: [
                {
                    journalId: "2026/2026.journal",
                    index: 0,
                    line: 3,
                    pattern: "CHASE CHECKING",
                    replacement: "assets:bank:checking",
                    regex: false,
                    forwarded: true,
                    editable: true,
                },
                {
                    journalId: "2026/2026.journal",
                    index: 1,
                    line: 4,
                    pattern: "AMEX EPAYMENT",
                    replacement: "liabilities:card:amex",
                    regex: false,
                    forwarded: true,
                    editable: true,
                },
            ],
        },
    ],
};

/** The text inputs the editor offers: two per editable alias row. */
const patternFields = (container: HTMLElement): NodeListOf<HTMLInputElement> => container.querySelectorAll('input[type="text"]');

beforeEach(async () => {
    await connectFakeEngine({"/api/aliases": ALIASES});
    // Loaded through `ensureListing` and not `reload`, so the panel's own
    // `onServerReady` finds the same (nonce, url) key already served and does
    // not fire a second, asynchronous load underneath the assertions.
    await aliasStore.ensureListing(FAKE_ENGINE, settings.serverNonce);
});

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT AliasPanel", () => {
    it("mounts without the self-feeding effect that froze the whole app", () => {
        // `flushSync` is belt and braces. Testing Library's `render` already
        // flushes, so the loop guard throws from inside it today — the other
        // tests below rely on that and do not flush. It is spelled out HERE
        // because this is the assertion that is about the throw: an effect that
        // throws outside the closure surfaces as an unhandled rejection blamed
        // on some later test, which is a miserable way to find out the app is
        // frozen.
        expect(() => {
            render(AliasPanel);
            flushSync();
        }).not.toThrow();
    });

    it("seeds the editor from the listing's chosen file", () => {
        // The other half of the same property. An effect that never ran would
        // also never loop, so "did not throw" alone is satisfiable by a panel
        // that does nothing.
        render(AliasPanel);

        expect(screen.getByTestId("imports-aliases")).toBeDefined();
        expect(screen.getByDisplayValue("CHASE CHECKING")).toBeDefined();
        expect(screen.getByDisplayValue("liabilities:card:amex")).toBeDefined();
    });

    it("keeps the rows already on screen when a new one is added", async () => {
        // What the latch is FOR. A seeding effect that re-runs on every reactive
        // tick — the failure mode one `===` away from the loop above — silently
        // discards whatever the user has done since the file loaded.
        const {container} = render(AliasPanel);
        expect(patternFields(container)).toHaveLength(4);

        await fireEvent.click(screen.getByTestId("imports-alias-add"));

        expect(patternFields(container)).toHaveLength(6);
        expect(screen.getByDisplayValue("CHASE CHECKING")).toBeDefined();
    });
});
