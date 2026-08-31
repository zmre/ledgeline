// The history strip beside the budget amount box.
//
// The claims worth pinning are all about what the strip refuses to imply: that a
// part-month is a whole one, and that "no complete period yet" is an average of
// zero. Both are ways a reference figure could quietly talk someone into the
// wrong budget.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import type {AccountReference} from "$lib/budget/types";
import {dec} from "$lib/domain/money";
import ReferenceStrip from "./ReferenceStrip.svelte";

const STYLES = new Map([["$", {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: null}]]);

function reference(over: Partial<AccountReference> = {}): AccountReference {
    return {
        account: "expenses:food",
        interval: "monthly",
        inverted: false,
        periods: [
            {key: "2026-05", label: "May 2026", start: "2026-05-01", end: "2026-05-31", complete: true, total: new Map([["$", dec(61200n, 2)]])},
            {key: "2026-06", label: "Jun 2026", start: "2026-06-01", end: "2026-06-30", complete: true, total: new Map([["$", dec(54800n, 2)]])},
            {key: "2026-07", label: "Jul 2026", start: "2026-07-01", end: "2026-07-15", complete: false, total: new Map([["$", dec(38900n, 2)]])},
        ],
        average: new Map([["$", dec(58000n, 2)]]),
        averagedPeriods: 2,
        ...over,
    };
}

function mount(over: Partial<AccountReference> = {}) {
    return render(ReferenceStrip, {props: {view: "data" as const, reference: reference(over), styles: STYLES}});
}

describe("COMPONENT ReferenceStrip", () => {
    it("shows every period, and the average of the complete ones", () => {
        mount();

        expect(screen.getByText("May 2026")).toBeTruthy();
        expect(screen.getByText("$612.00")).toBeTruthy();
        expect(screen.getByText("$548.00")).toBeTruthy();
        expect(screen.getByText("$389.00")).toBeTruthy();

        const average = screen.getByTestId("reference-average");
        expect(average.textContent).toContain("Average");
        expect(average.textContent).toContain("$580.00");
    });

    it("says how many periods the average covers, so it cannot be read as covering all of them", () => {
        // Three periods are on screen and the average is of two — without the
        // count, a reader would reasonably assume it included the month in
        // progress and budget low.
        mount();
        expect(screen.getByTestId("reference-average").textContent).toContain("of 2");
    });

    it("labels the running period rather than showing it as a whole one", () => {
        mount();
        expect(screen.getByText("so far")).toBeTruthy();
    });

    it("shows NO average when no period has finished", () => {
        // `averagedPeriods: 0` is an absence, not a zero. Printing "$0.00" here
        // would be a confident answer to a question nobody can answer yet.
        mount({averagedPeriods: 0, average: new Map()});
        expect(screen.queryByTestId("reference-average")).toBeNull();
    });

    it("shows an average of nothing when the periods really were empty", () => {
        // The other side of the same coin: two complete months in which nothing
        // was spent IS an average, and a real one.
        mount({averagedPeriods: 2, average: new Map()});
        const average = screen.getByTestId("reference-average");
        expect(average.textContent).toContain("of 2");
    });

    it("stays quiet when the fetch failed, rather than blocking the edit", () => {
        render(ReferenceStrip, {props: {view: "error" as const, reference: null, styles: STYLES}});
        expect(screen.queryByTestId("reference-average")).toBeNull();
        expect(screen.getByText(/Couldn't load recent activity/)).toBeTruthy();
    });
});
