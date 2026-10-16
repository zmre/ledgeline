// The header badge is only there when there is something to report. A clean
// journal shows no triangle at all, rather than a warning sign with no count.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";

const state = {count: 0, maxSeverity: null as "error" | "warning" | "info" | null, drawerOpen: false};

vi.mock("#lib/stores/problems.svelte.js", () => ({
    problems: {
        get count() {
            return state.count;
        },
        get maxSeverity() {
            return state.maxSeverity;
        },
        get drawerOpen() {
            return state.drawerOpen;
        },
        set drawerOpen(open: boolean) {
            state.drawerOpen = open;
        },
    },
}));

const {default: ProblemsBadge} = await import("./ProblemsBadge.svelte");

describe("COMPONENT ProblemsBadge", () => {
    it("renders nothing when there are no problems", () => {
        state.count = 0;
        state.maxSeverity = null;
        render(ProblemsBadge);

        expect(screen.queryByRole("button")).toBeNull();
    });

    it("renders the button with its count when there are problems", () => {
        state.count = 2;
        state.maxSeverity = "warning";
        render(ProblemsBadge);

        expect(screen.getByRole("button", {name: "2 problems"})).toBeTruthy();
        expect(screen.getByText("2")).toBeTruthy();
    });
});
