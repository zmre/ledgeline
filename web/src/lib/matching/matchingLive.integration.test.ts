// INTEGRATION: the transfer-matching wire (plan 35) against a live engine
// (`just test-integration`, serving fixtures/sample.journal as of 2026-07-08).
// sample.journal declares no clearing account, so this pins the CONTRACT —
// the engine's real bytes decode through the SPA's decoders, the rows and
// summary still agree, and the writes refuse before writing — without
// touching the fixture. The writes themselves are pinned in Rust
// (crates/ledgeline-server/tests/match_endpoints.rs) over temp journals.

import {describe, expect, it} from "vitest";
import {decodeJournalSummary, decodeRowsPage} from "#lib/api/journalViewDecode.js";
import {decodeMatches} from "#lib/api/matchDecode.js";
import {ConflictError, LedgelineApi, ValidationError} from "#lib/api/native.js";
import {decodeQueuePage} from "#lib/api/reviewDecode.js";

const apiUrl = process.env.LEDGELINE_API_URL;
const TODAY = "2026-07-08";
const ALL = {from: null, to: null, accounts: new Set<string>(), query: ""};

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live matching routes", () => {
    const api = (): LedgelineApi => new LedgelineApi(apiUrl ?? "");

    it("serves the proposals: no clearing account, nothing proposed", async () => {
        const page = decodeMatches(await api().matches({today: TODAY, window: 10, lookback: 0}));
        expect(page.clearing).toBe(false);
        expect(page.counts).toEqual({clear: 0, check: 0});
        expect(page.proposals).toEqual([]);
    });

    it("keeps rows and summary counting the same rows, with no transfers", async () => {
        const summary = decodeJournalSummary(await api().journalSummary(ALL, {today: TODAY}));
        const page = decodeRowsPage(await api().journalRows(ALL, {offset: 0, limit: 1000, today: TODAY}));
        expect(summary.transfers).toBe(0);
        expect(page.total).toBe(summary.total);
        expect(page.rows.every((row) => row.transfer === undefined && row.linked === undefined)).toBe(true);
    });

    it("refuses a stale link, and a pair that is not one, before writing", async () => {
        const recents = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 2}));
        const [first, second] = recents.items;
        if (first === undefined || second === undefined) throw new Error("sample.journal has two recent transactions");
        const named = (item: typeof first, fingerprint = item.fingerprint ?? "") => ({index: item.txn.index, posting: 0, fingerprint});
        await expect(api().linkMatch({a: named(first, "stale"), b: named(second)})).rejects.toBeInstanceOf(ConflictError);
        // Fresh, but not on a clearing account.
        await expect(api().linkMatch({a: named(first), b: named(second)})).rejects.toBeInstanceOf(ValidationError);
        await expect(api().linkAllMatches({pairs: []})).rejects.toBeInstanceOf(ValidationError);
        const after = decodeQueuePage(await api().reviewQueue("recents", {today: TODAY, limit: 2}));
        expect(after.items.map((item) => item.fingerprint)).toEqual(recents.items.map((item) => item.fingerprint));
    });
});
