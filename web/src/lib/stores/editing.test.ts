import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {editing} from "./editing.svelte";
import {settings} from "./settings.svelte";

/** `probeEditing` GETs /api/transactions: 404 = read-only server, anything else = writes available. */
const status = (code: number): Response => new Response("", {status: code});

function serve(reply: () => Promise<Response>): void {
    vi.stubGlobal("fetch", (input: RequestInfo | URL) =>
        String(input).endsWith("/version") ? Promise.resolve(new Response('"1.52"', {status: 200, headers: {"Content-Type": "application/json"}})) : reply()
    );
}

beforeEach(async () => {
    serve(() => Promise.resolve(status(405)));
    await settings.setServerUrl("http://engine");
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("UNIT editing.probe distinguishes 'the server said no' from 'we couldn't ask' (FE-5g)", () => {
    it("enables editing when the write route answers", async () => {
        await editing.probe();
        expect(editing.canEdit).toBe(true);
        expect(editing.probeError).toBeNull();
    });

    it("disables editing when the server genuinely has no write route", async () => {
        serve(() => Promise.resolve(status(404)));
        await editing.probe();
        expect(editing.canEdit).toBe(false);
        expect(editing.probeError).toBeNull();
    });

    it("keeps the last known answer when the probe cannot be delivered", async () => {
        await editing.probe();
        expect(editing.canEdit).toBe(true);

        // One dropped packet used to land in `catch { canEdit = false; }`, which
        // removed the Add button, the inline editors and the popup for the rest
        // of the session — no message, nothing to click, and no way back.
        serve(() => Promise.reject(new TypeError("network error")));
        await editing.probe();

        expect(editing.canEdit).toBe(true);
        expect(editing.probeError).not.toBeNull();
    });

    it("clears the failure once a probe gets through again", async () => {
        serve(() => Promise.reject(new TypeError("network error")));
        await editing.probe();
        expect(editing.probeError).not.toBeNull();

        serve(() => Promise.resolve(status(404)));
        await editing.probe();
        expect(editing.probeError).toBeNull();
        expect(editing.canEdit).toBe(false);
    });
});
