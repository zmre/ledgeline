import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import {DEFAULT_PALETTE, OTHER_LABEL} from "$lib/format/palette";
import type {PeriodReport, PeriodRow, PeriodRowKind} from "../types";
import {MAX_NAMED, NET_LABEL_LIMIT, netLabelIndices, otherKey, periodStack, splitBySign, stackCommodity, stackEmptyReason, stackParts} from "./periodStack";

/** `{$: 12.34}`-style amounts from whole dollars, exact. */
const usd = (n: number): MixedAmount => (n === 0 ? new Map() : new Map([["$", dec(Math.round(n * 100), 2)]]));
const eur = (n: number): MixedAmount => new Map([["EUR", dec(Math.round(n * 100), 2)]]);

function row(account: string, values: number[], kind?: PeriodRowKind): PeriodRow {
    const out: PeriodRow = {account, depth: account.split(":").length, values: values.map(usd)};
    if (kind !== undefined) out.kind = kind;
    return out;
}

function report(rows: PeriodRow[], totals: number[], buckets = totals.map((_, i) => `202${i}`)): PeriodReport {
    return {buckets, rows, totals: totals.map(usd)};
}

const keys = (stack: {entities: readonly {key: string}[]}): string[] => stack.entities.map((e) => e.key);

describe("UNIT periodStack — parts of a depth-clamped tree", () => {
    it("stacks the leaves, never a parent and its child together", () => {
        const parts = stackParts([row("assets", [300]), row("assets:bank", [100]), row("assets:broker", [200])]);
        expect(parts.map((p) => p.key)).toEqual(["assets:bank", "assets:broker"]);
    });

    it("adds a residual for a parent's own postings, computed exactly", () => {
        const parts = stackParts([row("assets", [350, 300]), row("assets:bank", [100, 100]), row("assets:broker", [200, 200])]);
        expect(parts.map((p) => p.key)).toEqual(["assets (own)", "assets:bank", "assets:broker"]);
        expect(parts[0].values).toEqual([usd(50), usd(0)]);
    });

    it("recurses: a grandchild's parent is a leaf's parent, not the root", () => {
        const parts = stackParts([row("assets", [100]), row("assets:bank", [100]), row("assets:bank:checking", [60]), row("assets:bank:savings", [40])]);
        expect(parts.map((p) => p.key)).toEqual(["assets:bank:checking", "assets:bank:savings"]);
    });

    it("carries the engine's side onto a leaf and onto a residual", () => {
        const parts = stackParts([row("liabilities", [-120], "liability"), row("liabilities:cc", [-100], "liability")]);
        expect(parts.map((p) => [p.key, p.kind])).toEqual([
            ["liabilities (own)", "liability"],
            ["liabilities:cc", "liability"],
        ]);
    });

    it("sums the parts to the depth-1 total, so nothing is double counted", () => {
        const rows = [row("assets", [350]), row("assets:bank", [100]), row("assets:broker", [200]), row("liabilities", [-80]), row("liabilities:cc", [-80])];
        const stack = periodStack(report(rows, [270]), DEFAULT_PALETTE, "$");
        const drawn = stack.series.reduce((total, s) => total + s.values[0], 0);
        expect(drawn).toBe(270);
    });
});

describe("UNIT periodStack — sides", () => {
    it("places net-worth rows by the engine's kind, not by sign", () => {
        const rows = [
            row("assets", [-50, 100], "asset"),
            row("assets:checking", [-50, 100], "asset"), // overdrawn in the first bucket
            row("liabilities", [30, -200], "liability"),
            row("liabilities:cc", [30, -200], "liability"), // a refund in the first bucket
        ];
        const stack = periodStack(report(rows, [-20, -100]), DEFAULT_PALETTE, "$");
        expect(stack.entities.map((e) => [e.key, e.side])).toEqual([
            ["assets:checking", "up"],
            ["liabilities:cc", "down"],
        ]);
        // ... and the SIGN still decides where each bucket's segment is drawn:
        // the overdraft is a negative asset segment, the refund a positive
        // liability one.
        expect(stack.series.map((s) => [s.key, s.values])).toEqual([
            ["assets:checking|+", [0, 100]],
            ["assets:checking|-", [-50, 0]],
            ["liabilities:cc|+", [30, 0]],
            ["liabilities:cc|-", [0, -200]],
        ]);
    });

    it("places kind-less rows by the sign of their window total", () => {
        const rows = [row("income:salary", [500, 500]), row("expenses:rent", [-300, -300]), row("assets:broker", [100, -400])];
        const stack = periodStack(report(rows, [300, -200]), DEFAULT_PALETTE, "$");
        expect(Object.fromEntries(stack.entities.map((e) => [e.key, e.side]))).toEqual({
            "income:salary": "up",
            "expenses:rent": "down",
            "assets:broker": "down",
        });
    });

    it("places a mixed row by its sign too", () => {
        const stack = periodStack(report([row("assets", [-10], "mixed")], [-10], ["2026"]), DEFAULT_PALETTE, "$");
        expect(stack.entities[0].side).toBe("down");
    });
});

describe("UNIT periodStack — splitting by sign", () => {
    it("gives an account that changes sign two uniformly signed halves sharing its colour", () => {
        const [plus, minus] = splitBySign([{key: "a", label: "a", color: "#111", side: "up", values: [5, -3, 0, 2]}]);
        expect(plus).toEqual({key: "a|+", entity: "a", label: "a", color: "#111", values: [5, 0, 0, 2]});
        expect(minus).toEqual({key: "a|-", entity: "a", label: "a", color: "#111", values: [0, -3, 0, 0]});
    });

    it("draws no empty half", () => {
        expect(splitBySign([{key: "a", label: "a", color: "#111", side: "up", values: [5, 1]}]).map((s) => s.key)).toEqual(["a|+"]);
    });
});

describe("UNIT periodStack — tail folding and colour", () => {
    /** `n` asset accounts sized n, n−1, … and `m` liabilities sized −m … */
    function many(n: number, m: number): PeriodReport {
        const rows = [
            row("assets", [0], "asset"),
            ...Array.from({length: n}, (_, i) => row(`assets:a${i}`, [100 * (n - i)], "asset")),
            row("liabilities", [0], "liability"),
            ...Array.from({length: m}, (_, i) => row(`liabilities:l${i}`, [-(10 * (m - i))], "liability")),
        ];
        // Parents equal to their children, so no residual joins in.
        rows[0].values = [usd(rows.slice(1, n + 1).reduce((s, r) => s + Number(r.values[0].get("$")?.m ?? 0n) / 100, 0))];
        rows[n + 1].values = [usd(rows.slice(n + 2).reduce((s, r) => s + Number(r.values[0].get("$")?.m ?? 0n) / 100, 0))];
        return report(rows, [0], ["2026"]);
    }

    it("keeps ~six names and folds each side's tail into its own (other)", () => {
        const stack = periodStack(many(10, 5), DEFAULT_PALETTE, "$");
        const named = stack.entities.filter((e) => e.label !== OTHER_LABEL);
        expect(named.length).toBe(MAX_NAMED);
        // The liabilities are an order of magnitude smaller, but still keep two names.
        expect(named.filter((e) => e.side === "down").map((e) => e.key)).toEqual(["liabilities:l0", "liabilities:l1"]);
        expect(stack.entities.filter((e) => e.label === OTHER_LABEL).map((e) => [e.key, e.color])).toEqual([
            [otherKey("up"), DEFAULT_PALETTE.other],
            [otherKey("down"), DEFAULT_PALETTE.other],
        ]);
        // The folded tail holds exactly what was left out: assets a4…a9 = 6+5+4+3+2+1 hundreds.
        expect(stack.entities.find((e) => e.key === otherKey("up"))?.values).toEqual([2100]);
    });

    it("never folds a tail of one — (other) standing for one account only hides its name", () => {
        const stack = periodStack(many(5, 2), DEFAULT_PALETTE, "$");
        expect(stack.entities.some((e) => e.label === OTHER_LABEL)).toBe(false);
        expect(stack.entities).toHaveLength(7);
    });

    it("assigns categorical slots in legend order, never a slot twice, never the tail colour to a name", () => {
        const stack = periodStack(many(10, 5), DEFAULT_PALETTE, "$");
        const named = stack.entities.filter((e) => e.label !== OTHER_LABEL).map((e) => e.color);
        expect(named).toEqual(DEFAULT_PALETTE.categorical.slice(0, named.length));
        expect(new Set(named).size).toBe(named.length);
    });

    it("keeps an account's colour identical in every bucket — one colour per entity, both halves", () => {
        const rows = [row("assets:bank", [100, -20, 300]), row("income:salary", [50, 50, 50])];
        const stack = periodStack(report(rows, [150, 30, 350]), DEFAULT_PALETTE, "$");
        const bank = stack.series.filter((s) => s.entity === "assets:bank");
        expect(bank).toHaveLength(2);
        expect(new Set(bank.map((s) => s.color)).size).toBe(1);
    });

    it("drops accounts that are zero throughout — they would spend a slot on nothing", () => {
        const stack = periodStack(report([row("assets:a", [0, 0]), row("assets:b", [5, 5])], [5, 5]), DEFAULT_PALETTE, "$");
        expect(keys(stack)).toEqual(["assets:b"]);
    });
});

describe("UNIT periodStack — commodity", () => {
    it("charts the commodity carrying the most figures, and names the rest", () => {
        const rows: PeriodRow[] = [
            {account: "assets:checking", depth: 2, values: [usd(10), usd(20), usd(30)]},
            {account: "assets:wise", depth: 2, values: [eur(5), new Map(), new Map()]},
        ];
        const r: PeriodReport = {buckets: ["a", "b", "c"], rows, totals: [new Map([...usd(10), ...eur(5)]), usd(20), usd(30)]};
        expect(stackCommodity(r, "X")).toEqual({commodity: "$", omitted: ["EUR"]});
        const stack = periodStack(r, DEFAULT_PALETTE, "X");
        expect(keys(stack)).toEqual(["assets:checking"]);
        expect(stack.net).toEqual([10, 20, 30]);
    });

    it("falls back when the report holds no commodity at all", () => {
        expect(stackCommodity({buckets: ["a"], rows: [], totals: [new Map()]}, "$")).toEqual({commodity: "$", omitted: []});
    });
});

describe("UNIT periodStack — net and labels", () => {
    it("takes the net from the engine's totals, not from the parts", () => {
        const stack = periodStack(report([row("assets:a", [5, 5])], [7, 9]), DEFAULT_PALETTE, "$");
        expect(stack.net).toEqual([7, 9]);
    });

    it("labels every net point up to the limit, and only the last past it", () => {
        expect(netLabelIndices(5)).toEqual([0, 1, 2, 3, 4]);
        expect(netLabelIndices(NET_LABEL_LIMIT)).toHaveLength(NET_LABEL_LIMIT);
        expect(netLabelIndices(12)).toEqual([11]);
        expect(netLabelIndices(0)).toEqual([]);
    });
});

describe("UNIT periodStack — empty and degenerate", () => {
    it("says there is nothing when there are no rows and no net", () => {
        expect(stackEmptyReason(periodStack({buckets: ["a", "b"], rows: [], totals: [new Map(), new Map()]}, DEFAULT_PALETTE, "$"))).toBe("no-data");
    });

    it("says nothing when every row is zero", () => {
        expect(stackEmptyReason(periodStack(report([row("assets:a", [0, 0])], [0, 0]), DEFAULT_PALETTE, "$"))).toBe("no-data");
    });

    it("refuses a single bucket — a number, not a trend", () => {
        expect(stackEmptyReason(periodStack(report([row("assets:a", [5])], [5], ["2026"]), DEFAULT_PALETTE, "$"))).toBe("single-bucket");
    });

    it("draws two buckets", () => {
        expect(stackEmptyReason(periodStack(report([row("assets:a", [5, 6])], [5, 6]), DEFAULT_PALETTE, "$"))).toBeNull();
    });
});
