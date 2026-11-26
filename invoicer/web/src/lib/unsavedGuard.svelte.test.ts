// The one unsaved-changes guard every editing page uses (TW-8), and the one
// live-reload hook: what is asked about, what goes on after Save or
// Discard, what a failed save says, and which change events reload.
import {flushSync} from "svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {changes, noteChange, onFilesChanged} from "./changes.svelte";
import {UnsavedGuard, type GuardOptions, type NavigationLike, type SaveOutcome} from "./unsavedGuard.svelte";

const HERE = new URL("http://localhost/app/catalog");

let dirty: boolean;
let outcome: SaveOutcome;
let saveError: string | null;
let conflict: boolean;
let navigate: ReturnType<typeof vi.fn<(href: string) => unknown>>;
let discarded: number;

function guard(over: Partial<GuardOptions> = {}): UnsavedGuard {
    return new UnsavedGuard(
        {
            dirty: () => dirty,
            save: () => Promise.resolve(outcome),
            discard: () => {
                discarded += 1;
                dirty = false;
            },
            saveError: () => saveError,
            conflict: () => conflict,
            noun: "catalog",
            ...over,
        },
        navigate,
        () => HERE
    );
}

function navigation(to: string | null, type = "link"): NavigationLike & {cancelled: () => boolean} {
    let cancelled = false;
    return {
        type,
        to: to === null ? null : {url: new URL(to, HERE)},
        cancel: () => (cancelled = true),
        cancelled: () => cancelled,
    };
}

beforeEach(() => {
    dirty = true;
    outcome = "saved";
    saveError = null;
    conflict = false;
    discarded = 0;
    navigate = vi.fn<(href: string) => unknown>();
});

describe("UnsavedGuard", () => {
    it("lets a navigation through when nothing is unsaved", () => {
        dirty = false;
        const g = guard();
        const nav = navigation("/app/invoices");
        g.beforeNavigate(nav);
        expect(nav.cancelled()).toBe(false);
        expect(g.pending).toBeNull();
    });

    it("holds a navigation to another page over unsaved edits, and asks", () => {
        const g = guard();
        const nav = navigation("/app/invoices?status=open#top");
        g.beforeNavigate(nav);
        expect(nav.cancelled()).toBe(true);
        expect(g.pending?.leaving).toBe(true);
        expect(g.error).toBeNull();
    });

    it("doesn't ask about the same page with another query (the New menu's ?new=1)", () => {
        const g = guard();
        const nav = navigation("/app/catalog?new=1");
        g.beforeNavigate(nav);
        expect(nav.cancelled()).toBe(false);
        expect(g.pending).toBeNull();
    });

    it("leaves closing the window to beforeunload: the browser's own prompt", () => {
        const g = guard();
        const leave = navigation(null, "leave");
        g.beforeNavigate(leave);
        expect(leave.cancelled()).toBe(false);
        expect(g.pending).toBeNull();
        const event = {preventDefault: vi.fn()};
        g.beforeUnload(event);
        expect(event.preventDefault).toHaveBeenCalledOnce();
        dirty = false;
        g.beforeUnload(event);
        expect(event.preventDefault).toHaveBeenCalledOnce();
    });

    it("Save, then goes where it was going; its own navigation isn't asked about", async () => {
        const g = guard();
        g.beforeNavigate(navigation("/app/invoices"));
        // Still dirty while it navigates (typed during the save): not held again.
        navigate.mockImplementation(() => {
            const again = navigation("/app/invoices");
            g.beforeNavigate(again);
            expect(again.cancelled()).toBe(false);
        });
        await g.save();
        expect(navigate).toHaveBeenCalledWith("/app/invoices");
        expect(g.pending).toBeNull();
    });

    it("a failed save keeps the dialog open with why: the save's error, a conflict, or a fallback", async () => {
        outcome = "failed";
        const g = guard();
        g.proceed(() => navigate("x"));
        saveError = "The disk is full";
        await g.save();
        expect(g.error).toBe("The disk is full");
        saveError = null;
        conflict = true;
        await g.save();
        expect(g.error).toBe("The catalog changed on disk; reload it or keep yours first.");
        conflict = false;
        await g.save();
        expect(g.error).toBe("It couldn't be saved.");
        expect(g.pending).not.toBeNull();
        expect(navigate).not.toHaveBeenCalled();
    });

    it("a save that asks something else first closes the dialog and goes nowhere", async () => {
        outcome = "asked";
        const g = guard();
        g.beforeNavigate(navigation("/app/invoices"));
        await g.save();
        expect(g.pending).toBeNull();
        expect(g.error).toBeNull();
        expect(navigate).not.toHaveBeenCalled();
    });

    it("Discard drops the edits, then goes on", async () => {
        const g = guard();
        g.beforeNavigate(navigation("/app/invoices"));
        await g.discard();
        expect(discarded).toBe(1);
        expect(navigate).toHaveBeenCalledWith("/app/invoices");
        expect(g.pending).toBeNull();
    });

    it("Cancel stays, edits and all", () => {
        const g = guard();
        g.beforeNavigate(navigation("/app/invoices"));
        g.cancel();
        expect(g.pending).toBeNull();
        expect(discarded).toBe(0);
        expect(navigate).not.toHaveBeenCalled();
    });

    it("proceed runs a step at once when clean, and asks first when not", async () => {
        const step = vi.fn();
        dirty = false;
        const g = guard();
        g.proceed(step);
        expect(step).toHaveBeenCalledOnce();
        dirty = true;
        g.proceed(step);
        expect(step).toHaveBeenCalledOnce();
        expect(g.pending?.leaving).toBe(false);
        await g.discard();
        expect(step).toHaveBeenCalledTimes(2);
    });

    it("a save cancelled while in flight doesn't go on afterwards", async () => {
        let finish: (o: SaveOutcome) => void = () => undefined;
        const g = guard({save: () => new Promise<SaveOutcome>((resolve) => (finish = resolve))});
        g.beforeNavigate(navigation("/app/invoices"));
        const saving = g.save();
        g.cancel();
        finish("saved");
        await saving;
        expect(navigate).not.toHaveBeenCalled();
    });

    it("a navigation off the app goes by its whole URL", async () => {
        const g = guard();
        g.beforeNavigate(navigation("https://example.com/help"));
        await g.discard();
        expect(navigate).toHaveBeenCalledWith("https://example.com/help");
    });
});

describe("onFilesChanged", () => {
    it("runs once per change event after it is set up, with the event's paths, never for one already seen", () => {
        noteChange(["before.md"]);
        const seen: string[][] = [];
        const stop = $effect.root(() => onFilesChanged((paths) => seen.push(paths)));
        flushSync();
        expect(seen).toEqual([]);
        noteChange(["invoices/108.md"]);
        flushSync();
        noteChange([]);
        flushSync();
        expect(seen).toEqual([["invoices/108.md"], []]);
        stop();
        noteChange(["after.md"]);
        flushSync();
        expect(seen).toHaveLength(2);
        expect(changes.paths).toEqual(["after.md"]);
    });

    it("doesn't re-run when state the reload reads changes", () => {
        const counter = $state({n: 0});
        let runs = 0;
        const stop = $effect.root(() =>
            onFilesChanged(() => {
                runs += 1;
                void counter.n;
            })
        );
        noteChange([]);
        flushSync();
        counter.n += 1;
        flushSync();
        expect(runs).toBe(1);
        stop();
    });
});
