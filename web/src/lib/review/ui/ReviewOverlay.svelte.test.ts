// The review overlay AT REST and while editing (plan 32 Phase 3 UX
// requirements): what a card shows, how the footer follows the form, the
// similar section, the suggestion chips, the two modes and their keys, and
// focus. Saves are `ReviewOverlay.save.svelte.test.ts` — the run store is a
// singleton, so the two live in separate files.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {queueItemWire, queuePageWire, reviewTxnWire, serveReview, similarWire, txnFixture} from "#lib/testing/reviewFixture.js";
import {accountField, card, footer, MOD, mountOverlay, press, primaryButton, settle, suggestionWire} from "#lib/testing/reviewOverlay.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import KeysProbe from "#lib/testing/fixtures/KeysProbe.svelte";
import {reviewLaunch, resetReviewLaunch} from "./launch.svelte";

const blue = (index: number, account = "expenses:unknown") => txnFixture(index, `SQ *BLUE BOTTLE ${index} OAKLAND`, account, "2026-10-12");
const COFFEE = suggestionWire("expenses:food:coffee", true);
const GROCERIES = suggestionWire("expenses:food:groceries", false);

function serve(options: {suggestions?: Record<string, unknown>[]; candidates?: Record<string, unknown>[]; items?: Record<string, unknown>[]} = {}) {
    return serveReview({
        queue: () =>
            queuePageWire(
                "unknowns",
                options.items ?? [queueItemWire(blue(1), {suggestions: options.suggestions ?? [COFFEE, GROCERIES]}), queueItemWire(blue(2))]
            ),
        similar: (index) => similarWire(index, index === 1 ? (options.candidates ?? []) : []),
    });
}

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

describe("COMPONENT ReviewOverlay card", () => {
    it("shows who and how much, and lands focus on the card, not a field", async () => {
        serve();
        await mountOverlay();
        expect(screen.getByTestId("review-payee").textContent).toContain("SQ *BLUE BOTTLE 1 OAKLAND");
        expect(screen.getByTestId("review-amount").textContent).toContain("$5.00");
        expect(screen.getByTestId("review-hero").textContent).toContain("Oct 12, 2026");
        expect(screen.getByTestId("review-position").textContent?.replace(/\s+/g, " ").trim()).toBe("1 of 2");
        expect(document.activeElement).toBe(card());
    });

    it("disables Reviewed while a posting is unknown, and enables it the moment the account changes", async () => {
        serve();
        await mountOverlay();
        expect(footer().dataset.state).toBe("clean-unknown");
        expect(primaryButton().disabled).toBe(true);
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        expect(footer().dataset.state).toBe("dirty");
        expect(primaryButton().disabled).toBe(false);
        expect(primaryButton().textContent).toContain("Mark Reviewed and Save");
    });

    it("offers Save (not reviewed) for a rename that leaves the account unknown", async () => {
        serve();
        await mountOverlay();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Blue Bottle"}});
        expect(footer().dataset.state).toBe("dirty-unknown");
        expect(primaryButton().textContent).toContain("Save");
        expect(primaryButton().className).toContain("btn-primary");
    });

    it("swaps the footer on the first change, and back after Revert or after undoing it by hand", async () => {
        serve();
        await mountOverlay();
        const description = screen.getByLabelText("Description") as HTMLInputElement;
        await fireEvent.input(description, {target: {value: "Blue Bottle"}});
        expect(screen.getByTestId("review-revert")).toBeTruthy();
        await fireEvent.input(description, {target: {value: "SQ *BLUE BOTTLE 1 OAKLAND"}});
        expect(screen.queryByTestId("review-revert")).toBeNull();
        expect(screen.getByTestId("review-skip")).toBeTruthy();
        await fireEvent.input(description, {target: {value: "Blue Bottle"}});
        await fireEvent.click(screen.getByTestId("review-revert"));
        await settle();
        expect(footer().dataset.state).toBe("clean-unknown");
        expect((screen.getByLabelText("Description") as HTMLInputElement).value).toBe("SQ *BLUE BOTTLE 1 OAKLAND");
    });

    it("shows a recents card that is not unknown with Reviewed enabled", async () => {
        serveReview({queue: () => queuePageWire("recents", [queueItemWire(txnFixture(5, "Safeway", "expenses:food:groceries"), {unknown: false})])});
        await mountOverlay("recents");
        expect(footer().dataset.state).toBe("clean");
        expect(primaryButton().disabled).toBe(false);
        expect(primaryButton().className).toContain("btn-success");
    });

    it("names a generated unknown posting's rule, read-only, and keeps Reviewed disabled", async () => {
        const generated = txnFixture(7, "Blue Bottle Coffee", "expenses:food:coffee");
        generated.postings.push({
            account: "expenses:unknown",
            amounts: [],
            status: "unmarked",
            comment: "",
            tags: [],
            generated: {rule: 0, query: "desc:coffee$"},
        });
        serveReview({queue: () => queuePageWire("unknowns", [queueItemWire(generated)])});
        await mountOverlay();
        expect(screen.getByTestId("review-generated").textContent).toContain("= desc:coffee$");
        expect(screen.getAllByTestId("review-posting")).toHaveLength(2);
        expect(primaryButton().disabled).toBe(true);
    });

    it("is a self-contained card: its footer (and the close bar) live inside it", async () => {
        serve();
        await mountOverlay();
        expect(card().contains(footer())).toBe(true);
        expect(card().contains(primaryButton())).toBe(true);
    });

    it("says there is nothing to review, with a close button", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [])});
        reviewLaunch.open("unknowns");
        await mountOverlay();
        expect(screen.getByTestId("review-empty").textContent).toContain("Nothing to review");
        await fireEvent.click(screen.getByRole("button", {name: "Close"}));
        expect(reviewLaunch.kind).toBeNull();
    });
});

describe("COMPONENT ReviewOverlay deck", () => {
    const three = () => [queueItemWire(blue(1)), queueItemWire(blue(2)), queueItemWire(blue(3))];
    const peeks = () => screen.queryAllByTestId("review-peek");

    it("stacks the next two cards beneath the top one: inert, hidden from assistive tech, hero only", async () => {
        serve({items: three()});
        await mountOverlay();
        expect(peeks().map((peek) => peek.dataset.peekIndex)).toEqual(["2", "3"]);
        const [next, after] = peeks() as [HTMLElement, HTMLElement];
        expect(next.textContent).toContain("SQ *BLUE BOTTLE 2 OAKLAND");
        expect(next.textContent).toContain("$5.00");
        expect(next.hasAttribute("inert")).toBe(true);
        expect(next.getAttribute("aria-hidden")).toBe("true");
        expect(next.querySelector("input, button")).toBeNull();
        // The nearest one sits higher in the pile, and both are pinned in the pile's slots.
        expect(next.getAttribute("style")).toContain("translateY(14px) scale(0.95)");
        expect(after.getAttribute("style")).toContain("translateY(28px) scale(0.9)");
        expect(next.className).toContain("z-[2]");
        expect(after.className).toContain("z-[1]");
        // The hero's ids still name only the top card.
        expect(screen.getByTestId("review-payee").textContent).toContain("SQ *BLUE BOTTLE 1 OAKLAND");
        expect(card().contains(screen.getByTestId("review-payee"))).toBe(true);
    });

    it("moves the pile up as cards leave, and shows no peek under the last card", async () => {
        serve({items: three()});
        await mountOverlay();
        await press("n");
        expect(peeks().map((peek) => peek.dataset.peekIndex)).toEqual(["3"]);
        expect(peeks()[0]?.getAttribute("style")).toContain("translateY(14px) scale(0.95)");
        await press("n");
        expect(card().dataset.cardIndex).toBe("3");
        expect(peeks()).toHaveLength(0);
        await press("ArrowLeft");
        expect(peeks().map((peek) => peek.dataset.peekIndex)).toEqual(["3"]);
    });
});

describe("COMPONENT ReviewOverlay similar section", () => {
    const siblings = () => [reviewTxnWire(blue(11)), reviewTxnWire(blue(12)), reviewTxnWire(blue(13, "expenses:food:groceries"))];

    it("is absent when nothing matches", async () => {
        serve({candidates: []});
        await mountOverlay();
        expect(screen.queryByTestId("review-similar")).toBeNull();
    });

    it("starts collapsed with its count, every row unchecked, and follows the change client-side", async () => {
        const seen = serve({candidates: siblings()});
        await mountOverlay();
        await vi.waitFor(() => expect(screen.getByTestId("review-similar")).toBeTruthy());
        const section = screen.getByTestId("review-similar");
        expect(section.dataset.open).toBeUndefined();
        expect(screen.getByTestId("review-similar-count").textContent).toBe("3");
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        // Only the two still on expenses:unknown can take an account change.
        expect(screen.getByTestId("review-similar-count").textContent).toBe("2");
        const boxes = screen.getAllByTestId("review-similar-row").map((row) => row.querySelector("input") as HTMLInputElement);
        expect(boxes.every((box) => !box.checked)).toBe(true);
        expect(screen.getAllByTestId("review-similar-chip").map((chip) => chip.textContent?.trim())).toEqual(["→ food:coffee", "→ food:coffee"]);
        // One fetch for the card, however many edits.
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Blue Bottle"}});
        expect(seen.filter((r) => r.url.pathname.endsWith("/similar/1"))).toHaveLength(1);
        expect(screen.getByTestId("review-similar-count").textContent).toBe("3");
    });

    it("has a tri-state header box, and ⇧A checks and unchecks all", async () => {
        serve({candidates: siblings()});
        await mountOverlay();
        await vi.waitFor(() => expect(screen.getByTestId("review-similar")).toBeTruthy());
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        card().focus();
        await press("s");
        expect(screen.getByTestId("review-similar").dataset.open).toBe("");
        const all = screen.getByTestId("review-similar-all") as HTMLInputElement;
        const first = screen.getAllByTestId("review-similar-row")[0]?.querySelector("input") as HTMLInputElement;
        await fireEvent.click(first);
        expect(all.indeterminate).toBe(true);
        expect(all.checked).toBe(false);
        card().focus();
        await press("A");
        expect(all.checked).toBe(true);
        expect(all.indeterminate).toBe(false);
        await press("A");
        expect(all.checked).toBe(false);
    });

    it("hides itself for a split and says why", async () => {
        serve({candidates: siblings()});
        await mountOverlay();
        await vi.waitFor(() => expect(screen.getByTestId("review-similar")).toBeTruthy());
        await fireEvent.input(screen.getByLabelText("Amount for posting 1"), {target: {value: "4.00"}});
        expect(screen.queryByTestId("review-similar")).toBeNull();
        expect(screen.getByTestId("review-split-note").textContent).toContain("Splits aren't applied to similar transactions");
    });
});

describe("COMPONENT ReviewOverlay suggestion chips", () => {
    it("shows the chips under the unknown row, the strong one selected", async () => {
        serve();
        await mountOverlay();
        const chips = screen.getAllByTestId("review-chip");
        expect(chips.map((chip) => chip.getAttribute("aria-pressed"))).toEqual(["true", "false"]);
        expect(chips[0]?.textContent).toContain("food:coffee");
    });

    it("fills the selected chip in on Tab, and cycles the alternatives on the next ones", async () => {
        serve();
        await mountOverlay();
        accountField().focus();
        const first = await press("Tab");
        expect(first.defaultPrevented).toBe(true);
        expect(accountField().value).toBe("expenses:food:coffee");
        const second = await press("Tab");
        expect(second.defaultPrevented).toBe(true);
        expect(accountField().value).toBe("expenses:food:groceries");
        expect(screen.getAllByTestId("review-chip")[1]?.getAttribute("aria-pressed")).toBe("true");
        await press("Tab");
        expect(accountField().value).toBe("expenses:food:coffee");
        expect(document.activeElement).toBe(accountField());
    });

    it("lets Tab leave the field once the user typed an account of their own", async () => {
        serve();
        await mountOverlay();
        accountField().focus();
        await fireEvent.input(accountField(), {target: {value: "expenses:rent"}});
        const event = await press("Tab");
        expect(accountField().value).toBe("expenses:rent");
        expect(event.defaultPrevented).toBe(false);
    });
});

describe("COMPONENT ReviewOverlay keys", () => {
    it("does nothing with single keys while a field is focused", async () => {
        serve();
        await mountOverlay();
        const description = screen.getByLabelText("Description") as HTMLInputElement;
        description.focus();
        await press("n");
        expect(review.cursor).toBe(0);
        await press("a");
        expect(document.activeElement).toBe(description);
    });

    it("walks Escape back out: field → card → close", async () => {
        serve();
        reviewLaunch.open("unknowns");
        await mountOverlay();
        accountField().focus();
        await press("Escape");
        expect(document.activeElement).toBe(card());
        expect(reviewLaunch.kind).toBe("unknowns");
        await press("Escape");
        expect(reviewLaunch.kind).toBeNull();
    });

    it("asks before closing a card with unsaved edits", async () => {
        serve();
        reviewLaunch.open("unknowns");
        await mountOverlay();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Blue Bottle"}});
        card().focus();
        await press("Escape");
        expect(card().contains(screen.getByTestId("review-confirm-close"))).toBe(true);
        expect(screen.queryByTestId("review-footer")).toBeNull();
        expect(reviewLaunch.kind).toBe("unknowns");
        await press("Escape");
        expect(screen.queryByTestId("review-confirm-close")).toBeNull();
        await press("Escape");
        await fireEvent.click(screen.getByRole("button", {name: "Discard and close"}));
        expect(reviewLaunch.kind).toBeNull();
    });

    it("jumps to fields: d, a (the unknown account), 1–9, $, t, m", async () => {
        serve();
        await mountOverlay();
        const cases: [string, string][] = [
            ["d", "Description"],
            ["a", "Account for posting 1"],
            ["2", "Account for posting 2"],
            ["$", "Amount for posting 1"],
            ["t", "Add a tag"],
            ["m", "Comment"],
        ];
        for (const [key, label] of cases) {
            card().focus();
            await press(key);
            expect(document.activeElement).toBe(screen.getByLabelText(label));
        }
    });

    it("skips with →, n and l, and goes back with ←, p and h, keeping a draft", async () => {
        serve();
        await mountOverlay();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Blue Bottle"}});
        card().focus();
        await press("ArrowRight");
        expect(review.cursor).toBe(1);
        expect(document.activeElement).toBe(card());
        await press("ArrowLeft");
        expect(review.cursor).toBe(0);
        expect((screen.getByLabelText("Description") as HTMLInputElement).value).toBe("Blue Bottle");
        await press("z");
        expect((screen.getByLabelText("Description") as HTMLInputElement).value).toBe("SQ *BLUE BOTTLE 1 OAKLAND");
        await press("n");
        expect(review.cursor).toBe(1);
        await press("p");
        expect(review.cursor).toBe(0);
        await press("l");
        expect(review.cursor).toBe(1);
        expect(document.activeElement).toBe(card());
        await press("h");
        expect(review.cursor).toBe(0);
        expect(screen.getByTestId("review-stack").dataset.exit).toBe("back");
    });

    it("lists h and l in the help sheet beside the arrows", async () => {
        serve();
        await mountOverlay();
        const rows = keymap.help.find((section) => section.group === "Review")?.rows ?? [];
        expect(rows.filter((row) => row.label === "Skip").map((row) => row.keys)).toEqual(["ArrowRight", "n", "l"]);
        expect(rows.filter((row) => row.label === "Previous card").map((row) => row.keys)).toEqual(["ArrowLeft", "p", "h"]);
    });

    it("blinds the journal's keys while it is open (a modal overlay layer)", async () => {
        const journalKey = vi.fn();
        render(KeysProbe, {
            props: {layerOf: () => ({id: "journal", priority: 10, bindings: [{keys: "j", label: "Next", group: "Journal" as const, run: journalKey}]})},
        });
        serve();
        await mountOverlay();
        card().focus();
        await press("j");
        expect(journalKey).not.toHaveBeenCalled();
        expect(keymap.help.map((section) => section.group)).toContain("Review");
        expect(keymap.help.map((section) => section.group)).not.toContain("Journal");
    });

    it("binds ⌘↵ in fields too: with the account still unknown it does nothing; once fixed it saves", async () => {
        serve();
        await mountOverlay();
        accountField().focus();
        const ignored = await press("Enter", MOD);
        expect(ignored.defaultPrevented).toBe(true);
        expect(review.cursor).toBe(0);
        // The save itself is covered in the .save file; here only that the
        // chord reached the overlay from inside a field.
    });
});
