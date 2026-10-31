// The editor's review badge (plan 32 D21) and its modal key layer (D16): with
// the popup up, the journal's single keys cannot fire behind it.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import KeysProbe from "#lib/testing/fixtures/KeysProbe.svelte";
import {wireRow} from "#lib/testing/journalViewFixture.js";
import {txnFixture} from "#lib/testing/reviewFixture.js";
import TransactionModal from "./TransactionModal.svelte";
import {txnModal} from "./modalState.svelte";

afterEach(() => {
    txnModal.close();
    vi.unstubAllGlobals();
    keymap.reset();
});

async function openWith(tags: [string, string][]): Promise<void> {
    const txn = {...txnFixture(4, "Safeway", "expenses:food"), tags};
    await connectFakeEngine({"/api/transactions/4": wireRow(txn)});
    txnModal.openEdit(4);
    await vi.waitFor(() => expect(txnModal.target).not.toBeNull());
    await tick();
}

describe("COMPONENT TransactionModal review badge", () => {
    it.each([
        [[["reviewed", "2026-10-30"]] as [string, string][], "Reviewed Oct 30"],
        [[["reviewed", "ignore"]] as [string, string][], "Ignored"],
        [[] as [string, string][], "Unreviewed"],
    ])("reads %j as %s", async (tags, label) => {
        render(TransactionModal);
        await openWith(tags);
        expect(screen.getByTestId("modal-review-badge").textContent?.trim()).toBe(label);
    });

    it("has no badge when adding", async () => {
        render(TransactionModal);
        txnModal.openAdd();
        await tick();
        expect(screen.queryByTestId("modal-review-badge")).toBeNull();
    });
});

describe("COMPONENT TransactionModal key layer", () => {
    it("blinds the journal's keys while open, and only then", async () => {
        const next = vi.fn();
        render(KeysProbe, {
            props: {layerOf: () => ({id: "journal", priority: 10, bindings: [{keys: "j", label: "Next", group: "Journal" as const, run: next}]})},
        });
        render(TransactionModal);
        const press = () => keymap.handle(new KeyboardEvent("keydown", {key: "j", cancelable: true}));
        press();
        expect(next).toHaveBeenCalledTimes(1);
        txnModal.openAdd();
        await tick();
        press();
        expect(next).toHaveBeenCalledTimes(1);
        txnModal.close();
        await tick();
        press();
        expect(next).toHaveBeenCalledTimes(2);
    });
});
