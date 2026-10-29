// INTEGRATION: `GET /api/auto-rules` and the journal rows against a live
// engine (`just test-integration`, which serves fixtures/sample.journal). That
// journal has no `=` rules and no hledger.conf, so this pins the CONTRACT —
// the live bytes decode through the SPA's decoders — and that a journal with
// no rules serves no generated posting. The applied case is pinned in Rust
// (crates/ledgeline-server/tests/auto_endpoints.rs) over fixtures/auto-postings.

import {describe, expect, it} from "vitest";
import {LedgelineApi} from "#lib/api/native.js";
import {decodeRowsPage} from "#lib/api/journalViewDecode.js";
import {decodeAutoRules} from "./decode";

const apiUrl = process.env.LEDGELINE_API_URL;

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live /api/auto-rules", () => {
    const url = apiUrl ?? "";

    it("decodes: no rules, --auto not in force", async () => {
        const listing = decodeAutoRules(await new LedgelineApi(url).listAutoRules());
        expect(listing.rules).toEqual([]);
        expect(listing.inForce).toBe(false);
        expect(listing.confHijacked).toBe(false);
    });

    it("serves no generated posting for a journal without rules", async () => {
        const page = decodeRowsPage(
            await new LedgelineApi(url).journalRows(
                {from: null, to: null, accounts: new Set(), query: ""},
                {offset: 0, limit: 500, order: "asc", today: "2026-07-08"}
            )
        );
        expect(page.rows.length).toBeGreaterThan(0);
        expect(page.rows.flatMap((row) => row.txn.postings).some((posting) => posting.generated !== undefined)).toBe(false);
    });
});
