// Cross-language parity: the TypeScript holdings pools (which back the Problems
// drawer's three stock rules) against the Rust holdings engine (which backs the
// Holdings page). Same journal, same as-of date, same declared account types.
//
// This is the test DRY-1 says does not exist. Until the TS engine is deleted the
// duplication stays, so the point of this file is to make the duplication
// DETECTABLE: every agreement below is asserted, and every remaining difference
// is asserted TOO — as the specific value each side produces, with the reason.
// Change either engine and this test fails and says which.
//
// Inputs are the four wire captures under fixtures/parity/, taken from the real
// binary serving fixtures/parity/holdings.journal, so the TS side is fed exactly
// what the SPA fetches. Regenerate all four together:
//
//   export LEDGELINE_TOKEN=parity-test-token-0123456789
//   cargo build --release
//   ./target/release/ledgeline --server --port 5137 fixtures/parity/holdings.journal &
//   for ep in transactions prices accounts; do
//     curl -sH "Authorization: Bearer $LEDGELINE_TOKEN" \
//       http://127.0.0.1:5137/$ep > fixtures/parity/$ep.json
//   done
//   curl -sH "Authorization: Bearer $LEDGELINE_TOKEN" \
//     "http://127.0.0.1:5137/api/holdings?asOf=2024-12-31" |
//     python3 -m json.tool > fixtures/parity/rust-holdings.json
//
// The capture in the repo came from a release build of 2026-07-27T23:26Z. The
// INTEGRATION block at the bottom re-checks it against a live server whenever
// LEDGELINE_API_URL is set — that is what catches a Rust-side change, so run it
// (or re-capture) after touching crates/ledgeline-core/src/holdings/.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {normalizeAccounts, normalizePrices, normalizeTransactions} from "../api/normalize";
import type {Dec} from "../domain/money";
import {computeHoldings} from "../holdings/engine";
import type {Holding} from "../holdings/types";
import {today} from "../reports/periods";
import {runChecks, type Problem} from "./engine";

/** As-of date pinned by the fixture: after every transaction and every P directive in it. */
const AS_OF = "2024-12-31";

interface WireDec {
    mantissa: string;
    places: number;
}
interface WireHolding {
    symbol: string;
    name: string;
    accounts: string[];
    shares: WireDec;
    basis: WireDec | null;
    firstBasisDate: string | null;
    price: {qty: WireDec; date: string; source: string} | null;
    marketValue: WireDec | null;
    gain: WireDec | null;
    gainPct: number | null;
}
interface WireHoldingsReport {
    asOf: string;
    base: string;
    holdings: WireHolding[];
    totals: {marketValue: WireDec; basis: WireDec | null; gain: WireDec | null; gainPct: number | null};
    topGainers: WireHolding[];
    topLosers: WireHolding[];
    warnings: {symbol: string; kind: string; message: string}[];
}

const load = (name: string): unknown => JSON.parse(readFileSync(new URL(`../../../../fixtures/parity/${name}`, import.meta.url), "utf8"));

const txns = normalizeTransactions(load("transactions.json"));
const prices = normalizePrices(load("prices.json"));
const decls = normalizeAccounts(load("accounts.json"));
const rust = load("rust-holdings.json") as WireHoldingsReport;

const ts = computeHoldings(txns, prices, {accounts: new Set(), mode: "include", asOf: AS_OF, gainPeriod: "all"}, decls);
const problems = runChecks(txns, {prices, decls});

/** A domain Dec in the engine's wire shape, so the two sides compare exactly (mantissa AND scale). */
const wire = (d: Dec | null): WireDec | null => (d === null ? null : {mantissa: d.m.toString(), places: d.p});

const tsBySymbol = new Map<string, Holding>(ts.holdings.map((h) => [h.symbol, h]));
const rustBySymbol = new Map<string, WireHolding>(rust.holdings.map((h) => [h.symbol, h]));

/**
 * The symbols the two engines fully agree on. Each exercises one shape the
 * `is_holding_account` filter exists for (FE-2): a share leg whose counter-side
 * is equity, revenue-by-name, or revenue-by-DECLARATION, plus an ordinary
 * purchase and a transfer between two holding accounts.
 *
 * ROUND joined this list once Rust adopted TS's taint reset: a cost-less lot
 * that is sold out IN FULL leaves nothing held whose cost is unknown, so the
 * shares bought back afterwards report their real basis. It was a documented
 * divergence (TS $1,000 / Rust null) until the Rust engine was fixed to match.
 */
const AGREED = ["OPEN", "VEST", "GRANT", "BUY", "XFER", "ROUND"] as const;

/**
 * Where the two still disagree, and why. Both engines are asserted against
 * BOTH columns below, so this list cannot rot: fix either side and the test
 * fails until this table is updated.
 */
const DIFFERENCES = [
    {
        symbol: "SPLIT",
        what: "2-for-1 split booked against equity:splits",
        // Rust HOLD-1 recognizes a re-denomination (no cost anywhere, only this
        // symbol moves, an equity leg absorbs the opposite sign) and scales the
        // share count while keeping the basis. TS has no split detection at all,
        // so the incoming 10 shares read as a cost-less acquisition.
        tsBasis: null,
        rustBasis: {mantissa: "100000", places: 2},
        tsWarns: true,
        rustWarns: false,
    },
] as const;

describe("UNIT checks/holdings TS↔Rust parity (fixtures/parity/holdings.journal)", () => {
    it("both engines see the same journal", () => {
        expect(rust.asOf).toBe(AS_OF);
        expect(ts.base).toBe(rust.base);
        expect(txns).toHaveLength(12);
        // The check RULES always value at today() (they take no as-of), so this
        // comparison is only meaningful while the fixture is entirely in the
        // past. Asserted rather than assumed, so a wrong clock says why.
        expect(today() > AS_OF).toBe(true);
        expect(decls.filter((d) => d.type !== null)).toHaveLength(7);
        // Every symbol is covered by exactly one of the two lists above.
        expect([...tsBySymbol.keys()].sort()).toEqual([...rustBySymbol.keys()].sort());
        expect([...AGREED, ...DIFFERENCES.map((d) => d.symbol)].sort()).toEqual([...rustBySymbol.keys()].sort());
    });

    it("agrees on row order (market value desc)", () => {
        expect(ts.holdings.map((h) => h.symbol)).toEqual(rust.holdings.map((h) => h.symbol));
    });

    it.each([...AGREED, ...DIFFERENCES.map((d) => d.symbol)])("agrees on %s's shares, price, market value, accounts and basis date", (symbol) => {
        const mine = tsBySymbol.get(symbol);
        const theirs = rustBySymbol.get(symbol);
        expect(mine).toBeDefined();
        expect(theirs).toBeDefined();
        expect(wire(mine!.shares)).toEqual(theirs!.shares);
        expect(wire(mine!.marketValue)).toEqual(theirs!.marketValue);
        expect(mine!.accounts).toEqual(theirs!.accounts);
        expect(mine!.firstBasisDate).toBe(theirs!.firstBasisDate);
        expect(mine!.name).toBe(theirs!.name);
        expect(mine!.price === null ? null : {qty: wire(mine!.price.qty), date: mine!.price.date, source: mine!.price.source}).toEqual(theirs!.price);
    });

    it.each(AGREED)("agrees on %s's basis and gain", (symbol) => {
        const mine = tsBySymbol.get(symbol)!;
        const theirs = rustBySymbol.get(symbol)!;
        expect(wire(mine.basis)).toEqual(theirs.basis);
        expect(wire(mine.gain)).toEqual(theirs.gain);
        if (mine.gainPct === null || theirs.gainPct === null) expect(mine.gainPct).toBe(theirs.gainPct);
        else expect(mine.gainPct).toBeCloseTo(theirs.gainPct, 10);
    });

    it("agrees on total market value — the shares themselves never diverge", () => {
        expect(wire(ts.totals.marketValue)).toEqual(rust.totals.marketValue);
    });

    it("agrees on the WARNING TEXT wherever both engines warn about a symbol", () => {
        const rustText = new Map(rust.warnings.map((w) => [`${w.symbol} ${w.kind}`, w.message]));
        const shared = ts.warnings.filter((w) => rustText.has(`${w.symbol} ${w.kind}`));
        expect(shared.map((w) => w.symbol)).toEqual(["GRANT", "VEST"]);
        for (const w of shared) expect(w.message).toBe(rustText.get(`${w.symbol} ${w.kind}`));
    });

    it("keeps the Problems drawer's stock rules in step with the TS engine's own warnings", () => {
        // The drawer runs the RULES; the (test-only) holdings report runs
        // computeHoldings. Both sit on buildPools, so their symbol sets must
        // match — this is the invariant FE-2 broke.
        const ruleSymbols = problems.filter((p: Problem) => p.rule === "stock-missing-basis").map((p) => p.message.split(" ")[0]);
        expect(ruleSymbols.sort()).toEqual(
            ts.warnings
                .filter((w) => w.kind === "missing-basis")
                .map((w) => w.symbol)
                .sort()
        );
        expect(problems.filter((p) => p.rule === "stock-negative" || p.rule === "stock-unpriced")).toEqual([]);
    });

    it("finds nothing unbalanced (the fixture is a journal hledger accepts)", () => {
        expect(problems.filter((p) => p.rule === "unbalanced")).toEqual([]);
    });
});

describe("UNIT checks/holdings TS↔Rust parity — the differences that REMAIN", () => {
    it.each(DIFFERENCES)("$symbol: $what", ({symbol, tsBasis, rustBasis, tsWarns, rustWarns}) => {
        expect(wire(tsBySymbol.get(symbol)!.basis)).toEqual(tsBasis);
        expect(rustBySymbol.get(symbol)!.basis).toEqual(rustBasis);
        expect(ts.warnings.some((w) => w.symbol === symbol && w.kind === "missing-basis")).toBe(tsWarns);
        expect(rust.warnings.some((w) => w.symbol === symbol && w.kind === "missing-basis")).toBe(rustWarns);
        // The drawer follows the TS engine, so it inherits the TS answer.
        expect(problems.some((p) => p.rule === "stock-missing-basis" && p.message.startsWith(`${symbol} `))).toBe(tsWarns);
    });

    it("totals: TS refuses a partial basis outright, Rust sums the rows it does know", () => {
        // Rust reports the basis of every untainted row — $1,000 OPEN + $1,000
        // SPLIT + $1,000 ROUND + $500 BUY + $250 XFER = $3,750 — while TS returns
        // null the moment ANY row is tainted, so the Holdings page and a
        // TS-rendered total read differently. (ROUND joined that sum when Rust
        // adopted the taint reset; SPLIT is still Rust-only, per DIFFERENCES.)
        expect(ts.totals.basis).toBeNull();
        expect(ts.totals.gain).toBeNull();
        expect(ts.totals.gainPct).toBeNull();
        expect(rust.totals.basis).toEqual({mantissa: "375000", places: 2});
        expect(rust.totals.gain).toEqual({mantissa: "52000", places: 2});
    });

    it("gainers/losers: same rule, different membership, because the basis differs", () => {
        // Both now rank ROUND. SPLIT is the only remaining asymmetry: Rust reads
        // the split and keeps its basis, TS taints it, and a null gain cannot be
        // ranked. Order is by gain descending, so ROUND lands last in both.
        expect(ts.topGainers.map((h) => h.symbol)).toEqual(["BUY", "OPEN", "XFER", "ROUND"]);
        expect(rust.topGainers.map((h) => h.symbol)).toEqual(["BUY", "OPEN", "SPLIT", "XFER", "ROUND"]);
        expect(ts.topLosers).toEqual([]);
        expect(rust.topLosers).toEqual([]);
    });

    it("documents the divergences this fixture does NOT exercise", () => {
        // Rust-only behaviour with no TS counterpart, deliberately kept out of
        // the shared fixture so the totals above stay comparable. Asserted as a
        // plain list so it is reviewed rather than forgotten; see
        // crates/ledgeline-core/src/holdings/engine.rs for each.
        expect([
            "shorts: Rust reports a negative position (negative market value in totals, basis/gain null); TS drops the row with a negative-shares warning",
            "sticky-negative taint (HOLD-4): a pool that dips below zero stays tainted in Rust; TS only taints on a cost-less lot",
            "TaintReason: Rust varies the warning text (cost-less lot / unconvertible cost / went negative); TS has one message for all three",
            "return of capital: Rust reduces the basis when cash is paid out of a single-security account; TS ignores it",
            "cost-incompatible round trips: Rust processes a zero-net sell-and-rebuy leg by leg; TS treats every zero net as a pure transfer",
            "valuation base: Rust picks the base by price coverage and honours ?valueIn; TS takes the price db's base commodity",
            "gain window: Rust supports gainSince/window_flow; TS has no windowed gain",
        ]).toHaveLength(7);
    });
});

// The frozen capture is only as good as its last regeneration. With a live
// server this block proves it still describes the engine — start one on the
// parity journal and run:
//   LEDGELINE_API_URL=http://127.0.0.1:5137 LEDGELINE_TOKEN=… vitest run parity
const apiUrl = process.env.LEDGELINE_API_URL;
const apiToken = process.env.LEDGELINE_TOKEN;

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION checks/holdings parity capture vs a live engine", () => {
    const get = async (path: string): Promise<unknown> => {
        const response = await fetch(`${apiUrl}${path}`, {headers: apiToken === undefined ? {} : {Authorization: `Bearer ${apiToken}`}});
        expect(response.ok, `${path} → ${response.status}`).toBe(true);
        return response.json();
    };

    it("still produces the captured holdings report", async () => {
        expect(await get(`/api/holdings?asOf=${AS_OF}`)).toEqual(rust);
    });

    it.each(["transactions", "prices", "accounts"])("still serves the captured /%s", async (endpoint) => {
        expect(await get(`/${endpoint}`)).toEqual(load(`${endpoint}.json`));
    });
});
