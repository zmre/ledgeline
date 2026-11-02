// The New Transactions screen brings what a press made appear into view
// (`#lib/ui/reveal`): the dry run after Test Import, the result after Write
// changes — and leaves a dry run that was already there when the screen
// mounted exactly where it is.
//
// jsdom has no layout, so the rects are stubbed: every revealed section reports
// itself far below the fold, and the assertion is on `window.scrollBy`. The
// arithmetic is `reveal.test.ts`'s; that the page really ends up showing the
// panel is `e2e/imports.e2e.ts`'s `toBeInViewport`.
//
// Its own file: `importStore` is a module singleton.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {importStore} from "#lib/imports/importStore.svelte.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {CAPABILITIES, STAGE, upload} from "#lib/testing/importFixtures.js";
import NewTransactionsPanel from "./NewTransactionsPanel.svelte";

const DRY_RUN = {
    ok: true,
    entries: "2026-06-24 ACME\n    expenses:unknown  $1\n    assets:bank\n",
    count: 1,
    status: "would import 1",
    skipped: null,
    balance: null,
    cliCommand: "ledgeline import -i bank.csv",
    idMatches: null,
    txns: [],
    records: null,
    overrides: 0,
    cliNote: null,
};

/** The section a test is watching, reported as below jsdom's 768px fold; everything else at the top. */
let below = "";
let scrollBy: ReturnType<typeof vi.fn>;

/** Let the reveal arm's after-layout flush run. */
const frame = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 30));

beforeEach(async () => {
    await connectFakeEngine({
        "/api/import/capabilities": CAPABILITIES,
        "/api/import/stage": STAGE,
        "/api/import/dry-run": DRY_RUN,
        "/api/import/commit": new Response(JSON.stringify({error: "the journal moved"}), {status: 500}),
    });
    await importStore.reloadCapabilities(FAKE_ENGINE);
    await importStore.offerFile(upload("bank.csv"));
    below = "";
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        const top = below !== "" && this.matches(`[data-testid="${below}"]`) ? 2000 : 0;
        return {top, height: 500, bottom: top + 500, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({})} as DOMRect;
    });
    scrollBy = vi.fn();
    vi.stubGlobal("scrollBy", scrollBy);
});

afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT NewTransactionsPanel — what a press makes appear is brought into view", () => {
    it("labels the dry-run button Test Import", () => {
        render(NewTransactionsPanel);
        expect(screen.getByTestId("imports-submit").textContent?.trim()).toBe("Test Import");
    });

    it("reveals the dry run after Test Import", async () => {
        render(NewTransactionsPanel);
        below = "imports-dry-run-reveal";

        await fireEvent.click(screen.getByTestId("imports-submit"));
        await vi.waitFor(() => expect(screen.getByTestId("imports-dry-run")).toBeDefined());
        await frame();

        expect(screen.getByTestId("imports-dry-run-reveal").dataset.reveal).toBe("");
        expect(scrollBy).toHaveBeenCalledWith(expect.objectContaining({top: expect.any(Number)}));
        expect(scrollBy.mock.calls.every(([options]) => ((options as ScrollToOptions).top ?? 0) > 0)).toBe(true);
    });

    it("reveals the result after Write changes — here, the write's error", async () => {
        render(NewTransactionsPanel);
        await fireEvent.click(screen.getByTestId("imports-submit"));
        await vi.waitFor(() => expect((screen.getByTestId("imports-write-changes") as HTMLButtonElement).disabled).toBe(false));
        await frame();
        scrollBy.mockClear();
        below = "imports-result-reveal";

        await fireEvent.click(screen.getByTestId("imports-write-changes"));
        await vi.waitFor(() => expect(screen.getByTestId("imports-commit-error")).toBeDefined());
        await frame();

        expect(scrollBy).toHaveBeenCalled();
    });

    it("leaves a dry run that was already on screen when the panel mounted where it is", async () => {
        await importStore.runDryRun();
        below = "imports-dry-run-reveal";

        render(NewTransactionsPanel);
        expect(screen.getByTestId("imports-dry-run")).toBeDefined();
        await frame();

        expect(scrollBy).not.toHaveBeenCalled();
    });
});
