// The three stock findings — stock-missing-basis, stock-negative,
// stock-unpriced — end to end: engine wire payload → normalizeDiagnostics →
// runChecks → the badge, the drawer and the per-row flags.
//
// THIS FILE REPLACES checks/parity.test.ts. That test existed to make a
// duplication detectable: the SPA computed these three from its own copy of the
// average-cost pools (web/src/lib/holdings/engine.ts) while the Holdings page
// computed them in Rust, and the two had drifted far enough to give opposite
// answers for the same journal. The parity test pinned, case by case, where they
// agreed and where they did not.
//
// There is nothing left to compare: the TS engine is deleted and the Rust one
// answers both pages. What has to be guarded now is the SEAM — that the engine's
// findings survive the wire, the rule allow-list, the index translation and the
// checks pipeline, and land on the same rows the deleted rules anchored them to.
// That is what the fixtures below are: real `/api/diagnostics` captures from the
// binary serving the same two journals. Regenerate them together:
//
//   export LEDGELINE_TOKEN=parity-test-token-0123456789
//   cargo build --release
//   ./target/release/ledgeline --server --port 5137 fixtures/parity/holdings.journal &
//   ./target/release/ledgeline --server --port 5138 fixtures/sample.journal &
//   curl -sH "Authorization: Bearer $LEDGELINE_TOKEN" http://127.0.0.1:5137/api/diagnostics |
//     python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin), indent=4, ensure_ascii=False))' \
//     > fixtures/parity/diagnostics.json
//   curl -sH "Authorization: Bearer $LEDGELINE_TOKEN" http://127.0.0.1:5138/api/diagnostics |
//     python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin), indent=4, ensure_ascii=False))' \
//     > fixtures/api/ledgeline/diagnostics.json
//
// The INTEGRATION block at the bottom re-checks the parity capture against a
// live server whenever LEDGELINE_API_URL is set; on the Rust side,
// crates/ledgeline-core/tests/stock_diagnostics.rs pins the same two journals
// against the engine directly, so a capture cannot go stale in silence.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {normalizeAccounts, normalizeDiagnostics, normalizePrices, normalizeTransactions} from "../api/normalize";
import type {Transaction} from "../domain/types";
import {groupByTxn, maxSeverity, runChecks, type Problem} from "./engine";

const load = (path: string): unknown => JSON.parse(readFileSync(new URL(`../../../../${path}`, import.meta.url), "utf8"));

/** The stock rules, in the order `/api/diagnostics` reports them. */
const STOCK_RULES = ["stock-missing-basis", "stock-negative", "stock-unpriced"];

const isStock = (problem: Problem): boolean => STOCK_RULES.includes(problem.rule);

/** One journal's captured wire payloads, run through the production pipeline. */
function pipeline(dir: string, prefix = "api/v1.52"): {txns: Transaction[]; problems: Problem[]} {
    const txns = normalizeTransactions(load(`fixtures/${prefix}/transactions.json`));
    const problems = runChecks(txns, {
        prices: normalizePrices(load(`fixtures/${prefix}/prices.json`)),
        decls: normalizeAccounts(load(`fixtures/${prefix}/accounts.json`)),
        diagnostics: normalizeDiagnostics(load(`fixtures/${dir}/diagnostics.json`), txns),
        engineChecked: true,
    });
    return {txns, problems};
}

const parity = pipeline("parity", "parity");
const sample = pipeline("api/ledgeline");

/** `txnIndex:rule` pairs, sorted — the anchoring contract in one comparable value. */
const pairs = (problems: Problem[]): string[] =>
    problems
        .filter(isStock)
        .map((p) => `${p.txnIndex}:${p.rule}`)
        .sort();

const dateOf = (txns: Transaction[], problem: Problem): string => txns.find((t) => t.index === problem.txnIndex)?.date ?? "?";

describe("UNIT checks stock diagnostics reach the drawer (fixtures/sample.journal)", () => {
    // These are the EXACT pairs the deleted TS rules produced, captured before
    // they were removed. `tindex` is 1-based, so the wire's 0-based 102/182
    // resolve to transactions 103 and 183.
    //
    // Both moved by three when plans/14 gave sample.journal two 2024-07-01
    // opening positions and a 2025-06-30 depreciation entry ahead of them. The
    // findings themselves are unchanged — same rules, same rows, same journal
    // defects — so this is a renumbering, not a behaviour change.
    it("anchors every finding to the same transaction the TS rules did", () => {
        expect(pairs(sample.problems)).toEqual(["103:stock-missing-basis", "103:stock-unpriced", "183:stock-negative"]);
    });

    it("flags the 2025-08-20 GLD gift as missing basis AND unpriced", () => {
        const gld = sample.problems.filter((p) => p.message.startsWith("GLD"));
        expect(gld.map((p) => p.rule).sort()).toEqual(["stock-missing-basis", "stock-unpriced"]);
        expect(gld.every((p) => dateOf(sample.txns, p) === "2025-08-20")).toBe(true);
    });

    it("flags the 2026-06-22 never-bought TSLA sell as negative shares", () => {
        const tsla = sample.problems.filter((p) => p.rule === "stock-negative");
        expect(tsla).toHaveLength(1);
        expect(tsla[0].message).toContain("TSLA");
        expect(dateOf(sample.txns, tsla[0])).toBe("2026-06-22");
    });

    it("keeps all three at severity `warning` — never promoted to error", () => {
        const stock = sample.problems.filter(isStock);
        expect(stock).not.toHaveLength(0);
        expect(stock.every((p) => p.severity === "warning")).toBe(true);
        // A journal whose only findings are stock ones must not light the badge red.
        expect(maxSeverity(stock)).toBe("warning");
    });

    it("reaches the per-row flags under the transaction's own 1-based index", () => {
        const byTxn = groupByTxn(sample.problems);
        expect(
            byTxn
                .get(103)
                ?.filter(isStock)
                .map((p) => p.rule)
        ).toEqual(["stock-missing-basis", "stock-unpriced"]);
        expect(
            byTxn
                .get(183)
                ?.filter(isStock)
                .map((p) => p.rule)
        ).toEqual(["stock-negative"]);
        // The 0-based wire positions must NOT survive the translation.
        expect(byTxn.get(102)?.some(isStock) ?? false).toBe(false);
        expect(byTxn.get(182)?.some(isStock) ?? false).toBe(false);
    });
});

describe("UNIT checks stock diagnostics (fixtures/parity/holdings.journal)", () => {
    it("reports the two cost-less lots that are still held, per lot", () => {
        expect(pairs(parity.problems)).toEqual(["2:stock-missing-basis", "3:stock-missing-basis"]);
        expect(parity.problems.filter(isStock).map((p) => p.message.split(":")[0])).toEqual(["GRANT", "VEST"]);
    });

    it("no longer flags the 2-for-1 SPLIT — the divergence DRY-1 was opened for", () => {
        // The TS pools had no split detection, so the incoming 10 shares read as
        // a cost-less acquisition and the drawer said "SPLIT lot acquired without
        // a cost annotation" while the Holdings page reported its real $1,000
        // basis. The Rust engine recognizes the re-denomination; deleting the TS
        // copy is what makes the two agree, because there is now only one.
        expect(parity.problems.some((p) => p.message.startsWith("SPLIT"))).toBe(false);
    });

    it("stays quiet about ROUND, closed out in full and re-bought at a known cost", () => {
        expect(parity.problems.some((p) => p.message.startsWith("ROUND"))).toBe(false);
    });
});

describe("UNIT checks no local rule computes a stock finding any more", () => {
    it("produces nothing without the engine's diagnostics", () => {
        // The whole point of DRY-1: the second engine is gone, so an SPA talking
        // to a backend with no /api/diagnostics route reports no stock findings
        // rather than wrong ones.
        const bare = runChecks(sample.txns, {prices: normalizePrices(load("fixtures/api/v1.52/prices.json"))});
        expect(bare.filter(isStock)).toEqual([]);
    });
});

// The frozen captures are only as good as their last regeneration. With a live
// server this block proves they still describe the engine:
//   LEDGELINE_API_URL=http://127.0.0.1:5137 LEDGELINE_TOKEN=… vitest run stock-diagnostics
const apiUrl = process.env.LEDGELINE_API_URL;
const apiToken = process.env.LEDGELINE_TOKEN;

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION stock diagnostics capture vs a live engine", () => {
    // WHICH journal the server holds is the caller's choice, so the capture to
    // compare against has to be discovered rather than assumed. CI and
    // `just test-integration` both serve fixtures/sample.journal; the parity
    // journal is what a DRY-1 regeneration run serves. Asserting one of them
    // unconditionally made this fail in CI against a perfectly good engine.
    const CAPTURES = ["fixtures/api/ledgeline/diagnostics.json", "fixtures/parity/diagnostics.json"];

    it("still serves the capture for whichever journal it is holding", async () => {
        const response = await fetch(`${apiUrl}/api/diagnostics`, {headers: apiToken === undefined ? {} : {Authorization: `Bearer ${apiToken}`}});
        expect(response.ok, `/api/diagnostics → ${response.status}`).toBe(true);
        const live = await response.json();

        const matched = CAPTURES.filter((path) => JSON.stringify(load(path)) === JSON.stringify(live));
        // Naming the served journal beats a bare deep-equal diff: the usual
        // cause of a miss is pointing at the wrong journal, not a stale capture.
        expect(
            matched,
            `live /api/diagnostics matched no committed capture. Serving an unexpected journal, or a capture is stale:\n${JSON.stringify(live, null, 2)}`
        ).not.toHaveLength(0);
    });
});

// ---------------------------------------------------------------------------
// The wire ⇄ SPA allow-list
// ---------------------------------------------------------------------------

describe("UNIT the engine's rule vocabulary matches the SPA allow-list", () => {
    // `normalizeDiagnostics` DROPS any rule it does not recognize, and drops it
    // SILENTLY — an engine finding the SPA has never heard of simply never
    // appears, with no error on either side to say so. Something has to make
    // widening the enum a two-sided change.
    //
    // This assertion lived in crates/ledgeline-core/tests/stock_diagnostics.rs
    // and read the TypeScript. It passed under `cargo test` and FAILED under
    // `nix build .#tests` — which is what CI runs — because that derivation's
    // source is `craneLib.cleanCargoSource`, so `web/` is not in it at all.
    // Reading in this direction always works: vitest runs from a full checkout.
    it("declares exactly the rules wire.rs does", () => {
        const wire = readFileSync(new URL("../../../../crates/ledgeline-core/src/wire.rs", import.meta.url), "utf8");
        const block = /pub const DIAGNOSTIC_RULES: \[&str; \d+\] = \[([^\]]*)\]/.exec(wire);
        expect(block, "wire.rs declares DIAGNOSTIC_RULES").not.toBeNull();
        const rust = [...block![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]).sort();

        const norm = readFileSync(new URL("../api/normalize.ts", import.meta.url), "utf8");
        const line = norm.split("\n").find((l) => l.includes("const DIAGNOSTIC_RULES"));
        expect(line, "normalize.ts declares DIAGNOSTIC_RULES").toBeDefined();
        const spa = [...line!.matchAll(/"([^"]+)"/g)].map((m) => m[1]).sort();

        expect(spa).toEqual(rust);
        expect(rust.length).toBeGreaterThanOrEqual(5); // guard against both regexes matching nothing
    });
});
