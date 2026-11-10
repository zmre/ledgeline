// The drawer's fixes for transfer matching's findings (plan 35 D10,
// Amendment A2): "in transit" offers Find match (jump to the row, then Find
// Matches focused on that transaction), "broken match" offers Unlink (asks
// first). Only on an editable journal, and only for those two rules.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import type {Problem} from "#lib/checks/engine.js";

const requestFocus = vi.fn();
const openFor = vi.fn();
const unlinkFrom = vi.fn(() => Promise.resolve({ok: true, value: {generation: "g2", id: "k7f3a2", edited: []}}));
const state = {problems: [] as Problem[], drawerOpen: true, canEdit: true};

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));
vi.mock("$app/state", () => ({page: {url: new URL("http://localhost/")}}));
vi.mock("#lib/stores/filters.svelte.js", () => ({
    filters: {value: {from: null, to: null}, setRange: vi.fn()},
}));
vi.mock("#lib/stores/editing.svelte.js", () => ({
    editing: {
        get canEdit() {
            return state.canEdit;
        },
    },
}));
vi.mock("#lib/stores/matches.svelte.js", () => ({matches: {openFor, unlinkFrom, busy: false}}));
vi.mock("#lib/stores/problems.svelte.js", () => ({
    problems: {
        get all() {
            return state.problems;
        },
        get count() {
            return state.problems.length;
        },
        get drawerOpen() {
            return state.drawerOpen;
        },
        set drawerOpen(open: boolean) {
            state.drawerOpen = open;
        },
        requestFocus,
    },
}));

const {default: ProblemsDrawer} = await import("./ProblemsDrawer.svelte");

const finding = (rule: string, txnIndex: number, message: string): Problem => ({
    txnIndex,
    date: "2026-10-02",
    description: "CAR PAYMENT",
    rule,
    severity: rule === "broken-match" ? "warning" : "info",
    message,
});
const IN_TRANSIT = finding("in-transit", 20, "$506.55 on transfers:car-payment has been in transit since 2026-10-02 with no matching transfer");
const BROKEN = finding("broken-match", 21, "match:k7f3a2 is on only one posting");
const UNUSUAL = finding("unusual-clearing", 22, "a clearing posting with a cost can never be matched");

function show(problems: Problem[], canEdit = true): void {
    state.problems = problems;
    state.drawerOpen = true;
    state.canEdit = canEdit;
    render(ProblemsDrawer);
}

beforeEach(() => {
    requestFocus.mockClear();
    openFor.mockClear();
    unlinkFrom.mockClear();
});

describe("COMPONENT ProblemsDrawer transfer-matching fixes", () => {
    it("labels the rules", () => {
        show([IN_TRANSIT, BROKEN, UNUSUAL]);
        expect(screen.getByText("In transit")).toBeDefined();
        expect(screen.getByText("Broken match")).toBeDefined();
        expect(screen.getByText("Unusual clearing posting")).toBeDefined();
    });

    it("offers Find match on an in-transit finding: jump to the row, then Find Matches focused on it", async () => {
        show([IN_TRANSIT]);
        await fireEvent.click(screen.getByTestId("problem-find-match"));
        await vi.waitFor(() => expect(openFor).toHaveBeenCalledWith(20));
        expect(requestFocus).toHaveBeenCalledWith(20);
        expect(state.drawerOpen).toBe(false);
    });

    it("offers Unlink on a broken match, after a confirm", async () => {
        show([BROKEN]);
        await fireEvent.click(screen.getByTestId("problem-unlink"));
        expect(unlinkFrom).not.toHaveBeenCalled();
        await fireEvent.click(screen.getByTestId("problem-unlink-go"));
        expect(unlinkFrom).toHaveBeenCalledWith(21, null);
    });

    it("offers no fix for other rules", () => {
        show([UNUSUAL]);
        expect(screen.queryByTestId("problem-fix")).toBeNull();
    });

    it("offers no fix on a read-only journal", () => {
        show([IN_TRANSIT, BROKEN], false);
        expect(screen.queryByTestId("problem-fix")).toBeNull();
    });
});
