// A list's search box (PLAN 5b): sent after a pause, at once on flush, and a
// URL value it didn't send replaces what it shows.
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {SearchBox} from "./searchBox.svelte.js";

describe("COMPONENT SearchBox", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("sends the trimmed text once typing pauses, and not the same text twice", () => {
        const send = vi.fn();
        const box = new SearchBox("", send, 200);
        box.type("cont");
        box.type("contoso ");
        vi.advanceTimersByTime(199);
        expect(send).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(send).toHaveBeenCalledExactlyOnceWith("contoso");
        box.flush();
        expect(send).toHaveBeenCalledOnce();
    });

    it("flushes at once, cancelling the pending send", () => {
        const send = vi.fn();
        const box = new SearchBox("", send, 200);
        box.type("105");
        box.flush();
        expect(send).toHaveBeenCalledExactlyOnceWith("105");
        vi.advanceTimersByTime(500);
        expect(send).toHaveBeenCalledOnce();
    });

    it("takes back its own value without touching the text, and a new one replaces it", () => {
        const send = vi.fn();
        const box = new SearchBox("", send, 200);
        box.type("acme ");
        box.flush();
        box.sync("acme");
        expect(box.text).toBe("acme ");
        box.sync("wingtip");
        expect(box.text).toBe("wingtip");
        box.type("x");
        box.sync("");
        vi.advanceTimersByTime(500);
        expect(send).toHaveBeenCalledOnce();
        expect(box.text).toBe("");
    });
});
