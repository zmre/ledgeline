// The Recognition overlay (plan 45 D6) against a fake engine: the Due
// table's months and tri-state checks, the closed row, Post through, the
// toast, Needs a schedule's Open in editor, and the button's gate.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {journal} from "#lib/stores/journal.svelte.js";
import {FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {dueEntryWire, dueMonthWire, dueWire, needsWire, serveAccrual, sideWire, tieWire} from "#lib/testing/accrualFixture.js";
import {accrual, resetAccrual} from "../store.svelte";
import RecognitionButton from "./RecognitionButton.svelte";
import RecognitionOverlay from "./RecognitionOverlay.svelte";

const OCT_X = dueEntryWire("cx1217", "2026-10-17", 100000);
const OCT_Y = dueEntryWire("cy0203", "2026-10-03", 83333, "Customer Y Recognized");
const NOV_X = dueEntryWire("cx1217", "2026-11-17", 100000);
const SEP_X = dueEntryWire("cx1217", "2026-09-17", 100000);

const DUE = dueWire(
    [
        dueMonthWire(
            "2026-10",
            [
                {party: "Customer X", entries: [OCT_X]},
                {party: "Customer Y", entries: [OCT_Y]},
            ],
            [tieWire(true)]
        ),
        dueMonthWire("2026-11", [{party: "Customer X", entries: [NOV_X]}], [tieWire(false)]),
    ],
    [SEP_X]
);

async function open(initial: Parameters<typeof serveAccrual>[0] = {}) {
    const served = await serveAccrual({due: DUE, ...initial});
    await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
    accrual.openOverlay();
    return served;
}

const month = (key: string): HTMLElement => screen.getAllByTestId("due-month").find((node) => node.dataset.month === key) as HTMLElement;
const postBodies = (seen: {method: string; url: URL; body?: unknown}[]) =>
    seen.filter((r) => r.method === "POST" && r.url.pathname.endsWith("/api/recognition/post")).map((r) => r.body as Record<string, unknown>);

beforeEach(() => resetAccrual());
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
    resetAccrual();
});

describe("COMPONENT RecognitionOverlay Due", () => {
    it("lists months with every open row checked, ties, and the closed row unchecked", async () => {
        await open();
        render(RecognitionOverlay);
        await vi.waitFor(() => expect(screen.getAllByTestId("due-month")).toHaveLength(2));
        const october = month("2026-10");
        expect(october.textContent).toContain("Oct 2026");
        expect(october.textContent).toContain("2 entries");
        expect(within(october).getByTestId("due-month-check").dataset.state).toBe("all");
        expect(within(october).getByTestId("due-tie").textContent).toContain("ties ✓");
        expect(within(month("2026-11")).getByTestId("due-tie").textContent).toContain("off by");
        expect(screen.getAllByTestId("due-entry-check").every((box) => (box as HTMLInputElement).checked)).toBe(true);
        expect((screen.getByTestId("due-closed-check") as HTMLInputElement).checked).toBe(false);
        expect(screen.getByTestId("due-closed").textContent).toContain("post as one adjustment today");
        expect(screen.getByTestId("recognition-post").textContent).toContain("Post through Nov 17");
    });

    it("turns a month some-checked, and posts exactly the checked rows through that month's end", async () => {
        const {seen} = await open();
        render(RecognitionOverlay);
        await vi.waitFor(() => expect(screen.getAllByTestId("due-month")).toHaveLength(2));
        // Uncheck November entirely, and Customer Y's October row.
        await fireEvent.click(within(month("2026-11")).getByTestId("due-month-check"));
        const october = month("2026-10");
        const yBox = within(october).getAllByTestId("due-entry-check")[1] as HTMLElement;
        await fireEvent.click(yBox);
        expect(within(october).getByTestId("due-month-check").dataset.state).toBe("some");
        expect(within(month("2026-11")).getByTestId("due-month-check").dataset.state).toBe("none");
        expect(screen.getByTestId("recognition-post").textContent).toContain("Post through Oct 31");
        await fireEvent.click(screen.getByTestId("recognition-post"));
        await vi.waitFor(() => expect(postBodies(seen)).toHaveLength(1));
        expect(postBodies(seen)[0]).toMatchObject({
            generation: "g1",
            through: "2026-10-31",
            entries: [{id: "cx1217", recognized: "2026-10-17"}],
            includeClosed: false,
        });
        await vi.waitFor(() => expect(screen.getByTestId("recognition-notice").textContent).toContain("Posted 1 recognition entry through 2026-10-31"));
        expect(screen.getByTestId("recognition-notice-link").textContent).toContain("Show Oct 2026");
    });

    it("folds the closed periods in when that row is checked", async () => {
        const {seen} = await open();
        render(RecognitionOverlay);
        await vi.waitFor(() => expect(screen.getByTestId("due-closed-check")).toBeDefined());
        await fireEvent.click(screen.getByTestId("due-closed-check"));
        await fireEvent.click(screen.getByTestId("recognition-post"));
        await vi.waitFor(() => expect(postBodies(seen)).toHaveLength(1));
        expect(postBodies(seen)[0]?.includeClosed).toBe(true);
        expect((postBodies(seen)[0]?.entries as unknown[]).length).toBe(3);
    });

    it("shows an entry's postings on expand, and says when nothing is due", async () => {
        await open();
        const {unmount} = render(RecognitionOverlay);
        await vi.waitFor(() => expect(screen.getAllByTestId("due-entry").length).toBe(3));
        await fireEvent.click(within(screen.getAllByTestId("due-entry")[0] as HTMLElement).getByRole("button", {name: "Show postings"}));
        expect(screen.getByTestId("due-entry-postings").textContent).toContain("liabilities:deferred-revenue");
        unmount();
        resetAccrual();
        await open({due: dueWire([])});
        render(RecognitionOverlay);
        await vi.waitFor(() => expect(screen.getByTestId("recognition-empty").textContent).toBe("Nothing is due."));
        expect((screen.getByTestId("recognition-post") as HTMLButtonElement).disabled).toBe(true);
    });

    it("says why a post was refused", async () => {
        const {state} = await open();
        render(RecognitionOverlay);
        await vi.waitFor(() => expect(screen.getAllByTestId("due-month")).toHaveLength(2));
        state.failNext = {status: 400, error: "2026-12-01 is after today"};
        await fireEvent.click(screen.getByTestId("recognition-post"));
        await vi.waitFor(() => expect(screen.getByTestId("recognition-failure").textContent).toContain("2026-12-01 is after today"));
    });
});

describe("COMPONENT RecognitionOverlay tabs", () => {
    it("lists Needs a schedule and opens one in the editor, closing first", async () => {
        await open({needs: needsWire([sideWire(41, {account: "liabilities:deferred-revenue", description: "Fourth Coffee Invoice 2004"})])});
        const onEdit = vi.fn();
        render(RecognitionOverlay, {onEdit});
        await fireEvent.click(screen.getByTestId("recognition-tab-needs"));
        await vi.waitFor(() => expect(screen.getByTestId("needs-item").textContent).toContain("Fourth Coffee Invoice 2004"));
        await fireEvent.click(screen.getByTestId("needs-edit"));
        expect(onEdit).toHaveBeenCalledWith(41);
        expect(accrual.open).toBe(false);
    });

    it("switches tabs with 1–3 and closes on Escape", async () => {
        await open();
        render(RecognitionOverlay);
        keymap.handle(new KeyboardEvent("keydown", {key: "3", cancelable: true}));
        expect(accrual.tab).toBe("invoices");
        await vi.waitFor(() => expect(screen.getByTestId("settlements-panel")).toBeDefined());
        keymap.handle(new KeyboardEvent("keydown", {key: "Escape", cancelable: true}));
        expect(accrual.open).toBe(false);
    });
});

describe("COMPONENT RecognitionButton", () => {
    it("shows the due count and opens the overlay, and renders nothing with the helpers off", async () => {
        await serveAccrual();
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        const {unmount} = render(RecognitionButton);
        expect(screen.getByTestId("recognition-count").textContent).toBe("3");
        expect(screen.getByTestId("recognition-reason").textContent).toContain("liabilities:deferred-revenue is declared a deferred revenue");
        await fireEvent.click(screen.getByTestId("recognition-button"));
        expect(accrual.open).toBe(true);
        unmount();
        resetAccrual();
        await serveAccrual({status: {enabled: false}});
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        render(RecognitionButton);
        expect(screen.queryByTestId("recognition-button")).toBeNull();
    });

    it("calls onOpen when given", async () => {
        await serveAccrual();
        await accrual.sync(FAKE_ENGINE, journal.generation ?? "");
        const onOpen = vi.fn();
        render(RecognitionButton, {onOpen});
        await fireEvent.click(screen.getByTestId("recognition-button"));
        expect(onOpen).toHaveBeenCalledOnce();
        expect(accrual.open).toBe(false);
    });
});
