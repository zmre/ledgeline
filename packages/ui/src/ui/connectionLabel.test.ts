// The app bar's four states, which are four different answers to "which ledger
// am I looking at": the engine named it; the engine cannot name it (a plain
// hledger-web, whose `/api/journal` 404s); there is no server at all; the
// connection is broken.

import {describe, expect, it} from "vitest";
import {connectionLabel, connectionTooltip} from "./connectionLabel";

const URL_A = "http://127.0.0.1:5000";

describe("UNIT connectionLabel — the visible label", () => {
    it("shows the journal's title once the engine has named it", () => {
        expect(connectionLabel("ready", "Acme Books")).toBe("Acme Books");
    });

    it("keeps showing the title through a poll and through an error, so the label does not flicker", () => {
        // Only "none" is about the server; the other four are `journal.status`,
        // and a reload or a failed poll does not change WHICH ledger is on
        // screen. The dot carries that news, and the tooltip carries the reason.
        for (const state of ["idle", "loading", "error"] as const) {
            expect(connectionLabel(state, "Acme Books")).toBe("Acme Books");
        }
    });

    it("shows NOTHING, never the server URL, when the engine derived no title", () => {
        // A plain hledger-web, or an engine older than /api/journal. The URL is
        // not a lesser answer to "which ledger is this" — it answers a different
        // question, and this spot is not where that question gets asked. The
        // layout renders no span at all for "", so the dot stands alone.
        expect(connectionLabel("ready", null)).toBe("");
    });

    it("treats a blank title as no title (a journal whose first line is a bare `;`)", () => {
        expect(connectionLabel("ready", "   ")).toBe("");
        expect(connectionLabel("ready", "")).toBe("");
    });

    it("trims a title rather than rendering the engine's whitespace", () => {
        expect(connectionLabel("ready", "  Acme Books  ")).toBe("Acme Books");
    });

    it("says `not connected` when there is no server, whatever a previous engine called its journal", () => {
        // The store clears the title on a round that could not fetch one, but
        // this branch must not depend on that having happened.
        expect(connectionLabel("none", "Acme Books")).toBe("not connected");
        expect(connectionLabel("none", null)).toBe("not connected");
    });
});

describe("UNIT connectionLabel — the hover tooltip", () => {
    it("pairs the journal file with the server URL, so where-am-I-connected survives the relabel", () => {
        expect(connectionTooltip("ready", "2026.journal", URL_A, null)).toBe(`2026.journal — ${URL_A}`);
    });

    it("falls back to the URL alone when the engine sent no file name", () => {
        expect(connectionTooltip("ready", null, URL_A, null)).toBe(URL_A);
        expect(connectionTooltip("ready", "  ", URL_A, null)).toBe(URL_A);
    });

    it("gives the whole tooltip to the error message while the connection is broken", () => {
        // The visible label beside a red dot is a ledger name; this sentence is
        // the only thing on screen that explains the Reconnect button.
        const message = `Cannot reach the Ledgeline engine at ${URL_A} (network or CORS failure)`;
        expect(connectionTooltip("error", "2026.journal", URL_A, message)).toBe(message);
    });

    it("still says something when the error state carries no message", () => {
        expect(connectionTooltip("error", "2026.journal", URL_A, null)).toBe("connection error");
    });

    it("explains the unconfigured case rather than hovering blank", () => {
        expect(connectionTooltip("none", null, null, null)).toBe("No hledger-web server configured");
    });
});
