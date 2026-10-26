// `pollWhileVisible` — the cadence both the journal's live updates and the
// Projections tab's open-file check run on. A `.svelte.test.ts` only for the
// jsdom project: it needs `document` and `window`.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {pollWhileVisible} from "./visiblePoll";

let visibility: DocumentVisibilityState = "visible";

beforeEach(() => {
    vi.useFakeTimers();
    visibility = "visible";
    vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
});

afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
});

function setVisibility(next: DocumentVisibilityState): void {
    visibility = next;
    document.dispatchEvent(new Event("visibilitychange"));
}

describe("COMPONENT pollWhileVisible", () => {
    it("runs on every interval while visible", () => {
        const run = vi.fn();
        const stop = pollWhileVisible(run, 1_000);
        vi.advanceTimersByTime(3_000);
        expect(run).toHaveBeenCalledTimes(3);
        stop();
    });

    it("stops while hidden, and runs at once on becoming visible again", () => {
        const run = vi.fn();
        const stop = pollWhileVisible(run, 1_000);
        setVisibility("hidden");
        vi.advanceTimersByTime(5_000);
        expect(run).not.toHaveBeenCalled();

        setVisibility("visible");
        expect(run).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(1_000);
        expect(run).toHaveBeenCalledTimes(2);
        stop();
    });

    it("runs at once when the window regains focus — coming back from an editor, which never hid it", () => {
        const run = vi.fn();
        const stop = pollWhileVisible(run, 60_000);
        window.dispatchEvent(new Event("focus"));
        expect(run).toHaveBeenCalledTimes(1);
        stop();
    });

    it("does nothing at all once stopped", () => {
        const run = vi.fn();
        pollWhileVisible(run, 1_000)();
        vi.advanceTimersByTime(5_000);
        window.dispatchEvent(new Event("focus"));
        setVisibility("visible");
        expect(run).not.toHaveBeenCalled();
    });
});
