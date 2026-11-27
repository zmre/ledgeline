// The tracker store: a toggle lights at once and the write follows; a
// refused write puts the state back with a message; a 409 the books moved
// under re-reads instead; an older answer never overwrites a newer press;
// the live clock; the guard's Keep all; pins and the person saved to
// ui-prefs.
import {describe, expect, it, vi} from "vitest";
import {EditorApiError} from "#lib/editorApi.js";
import {answer, fakeApi, running, trackState} from "./fixtures.js";
import {Tracker} from "./tracker.svelte.js";

/** A promise and its resolvers, to hold a request open. */
function held<T>(): {promise: Promise<T>; resolve: (value: T) => void; reject: (error: unknown) => void} {
    let resolve!: (value: T) => void;
    let reject!: (error: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return {promise, resolve, reject};
}

async function loaded(api = fakeApi(), clock = () => 1_000_000): Promise<Tracker> {
    const tracker = new Tracker(api, clock);
    await tracker.refresh();
    return tracker;
}

describe("COMPONENT the tracker store", () => {
    it("lights the slot before the write answers, then adopts the answer", async () => {
        const api = fakeApi();
        const write = held<ReturnType<typeof answer>>();
        api.toggle.mockReturnValueOnce(write.promise);
        const tracker = await loaded(api);
        const done = tracker.toggle(2);
        expect(tracker.state?.running?.account).toBe("clients:acme:advisory");
        expect(tracker.slot(2)?.running).toBe(true);
        expect(api.toggle).not.toHaveBeenCalled();
        await Promise.resolve();
        expect(api.toggle).toHaveBeenCalledWith({slot: 2});
        write.resolve(answer(running(trackState(), 2, 3, "2026-11-26T15:00:00")));
        expect(await done).toBe(true);
        expect(tracker.state?.running?.since).toBe("2026-11-26T15:00:00");
    });

    it("puts the state back with a message when the write is refused", async () => {
        const api = fakeApi();
        api.toggle.mockRejectedValueOnce(new EditorApiError(422, "verify", "the file didn't read back as written"));
        const tracker = await loaded(api);
        expect(await tracker.toggle(3)).toBe(false);
        expect(tracker.state?.running).toBeNull();
        expect(tracker.message).toBe("The timer couldn't be switched. The file didn't read back as written");
    });

    it("re-reads the state when the books moved under the write", async () => {
        let current = trackState();
        const api = fakeApi(() => current);
        const tracker = await loaded(api);
        // Another machine started slot 4 meanwhile.
        current = running(trackState(), 4, 60);
        api.toggle.mockRejectedValueOnce(new EditorApiError(409, "external-change", "the time file changed on disk"));
        await tracker.toggle(3);
        expect(api.state).toHaveBeenCalledTimes(2);
        expect(tracker.state?.running?.slot).toBe(4);
        expect(tracker.message).toContain("changed on disk");
    });

    it("sends presses in order and keeps the latest press on screen", async () => {
        const api = fakeApi();
        const first = held<ReturnType<typeof answer>>();
        const second = held<ReturnType<typeof answer>>();
        api.toggle.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
        const tracker = await loaded(api);
        const one = tracker.toggle(1);
        const three = tracker.toggle(3);
        expect(tracker.state?.running?.slot).toBe(3);
        first.resolve(answer(running(trackState(), 1)));
        await one;
        // Slot 1's answer is older than the press on 3: not shown.
        expect(tracker.state?.running?.slot).toBe(3);
        expect(api.toggle).toHaveBeenCalledTimes(2);
        second.resolve(answer(running(trackState(), 3, 0, "2026-11-26T15:00:01")));
        await three;
        expect(tracker.state?.running?.since).toBe("2026-11-26T15:00:01");
    });

    it("counts the running timer from when its state arrived", async () => {
        let now = 1_000_000;
        const tracker = await loaded(
            fakeApi(() => running(trackState(), 2, 600)),
            () => now
        );
        expect(tracker.elapsed).toBe(600);
        now += 5_000;
        tracker.tick();
        expect(tracker.elapsed).toBe(605);
        expect(tracker.todaySeconds).toBe(4500 + 605);
    });

    it("stops (or discards, or stops at a time) through the stop route", async () => {
        const api = fakeApi(() => running(trackState(), 2, 600));
        const tracker = await loaded(api);
        api.state.mockImplementation(async () => trackState());
        api.stop.mockResolvedValue(answer(trackState()));
        await tracker.stop({discard: true});
        expect(api.stop).toHaveBeenLastCalledWith({discard: true});
        // Nothing runs now: a second stop asks nothing.
        expect(await tracker.stop()).toBe(false);
        expect(api.stop).toHaveBeenCalledTimes(1);
    });

    it("asks the guard once per timer: Keep all quiets it", async () => {
        const guarded = {
            ...running(trackState(), 2, 5 * 3600),
            guard: {age: {seconds: 18000, hours: "5", text: "5h"}, previous_day: false, threshold_minutes: 240, last_activity: null},
        };
        const tracker = await loaded(fakeApi(() => guarded));
        expect(tracker.guard?.threshold_minutes).toBe(240);
        tracker.keepRunning();
        expect(tracker.guard).toBeNull();
    });

    it("saves a pin in its place among five, trailing empties left off", async () => {
        const api = fakeApi();
        const tracker = await loaded(api);
        await tracker.savePin(4, "internal:sales");
        expect(api.savePrefs).toHaveBeenLastCalledWith({
            pinned: ["clients:acme:tax-return", "clients:acme:advisory", "clients:globex:audit", "internal:admin", "internal:sales"],
        });
        await tracker.savePin(3, "");
        expect(api.savePrefs).toHaveBeenLastCalledWith({pinned: ["clients:acme:tax-return", "clients:acme:advisory", "clients:globex:audit"]});
    });

    it("saves the person, or none", async () => {
        const api = fakeApi();
        const tracker = await loaded(api);
        await tracker.savePerson("  sam ");
        expect(api.savePrefs).toHaveBeenLastCalledWith({person: "sam"});
        await tracker.savePerson("");
        expect(api.savePrefs).toHaveBeenLastCalledWith({person: null});
    });

    it("says why when the first read fails, and keeps what it has after", async () => {
        const api = fakeApi();
        api.state.mockRejectedValueOnce(new Error("no books are open"));
        const tracker = new Tracker(api);
        await tracker.refresh();
        expect(tracker.loadError).toBe("No books are open");
        await tracker.refresh();
        expect(tracker.loadError).toBeNull();
        const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
        api.state.mockRejectedValueOnce(new Error("gone"));
        await tracker.refresh();
        expect(tracker.state).not.toBeNull();
        warn.mockRestore();
    });
});
