import {describe, expect, it} from "vitest";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {ReportRow} from "$lib/reports/types";
import {compressPeriodRows, compressSectionRows} from "./displayRows";

const usd = (cents: number): MixedAmount => new Map([["$", dec(cents, 2)]]);
const zero = (): MixedAmount => new Map();

const row = (account: string, ownCents: number, inclusiveCents: number): ReportRow => ({
    account,
    depth: account.split(":").length,
    own: ownCents === 0 ? zero() : usd(ownCents),
    inclusive: usd(inclusiveCents),
});

describe("UNIT reports/ui/displayRows", () => {
    describe("compressSectionRows", () => {
        it("collapses single-child chains whose parents have no own postings", () => {
            const rows = [row("assets", 0, 500), row("assets:bank", 0, 500), row("assets:bank:checking", 500, 500)];
            expect(compressSectionRows(rows)).toEqual([{label: "assets:bank:checking", indent: 0, row: rows[2]}]);
        });

        it("keeps a parent with its own postings even when it has one child", () => {
            const rows = [row("assets", 100, 600), row("assets:bank", 500, 500)];
            const display = compressSectionRows(rows);
            expect(display.map((d) => [d.label, d.indent])).toEqual([
                ["assets", 0],
                ["bank", 1],
            ]);
        });

        it("keeps a parent with multiple children and labels children relative to it", () => {
            const rows = [
                row("assets", 0, 900),
                row("assets:bank", 0, 700),
                row("assets:bank:checking", 300, 300),
                row("assets:bank:savings", 400, 400),
                row("assets:cash", 200, 200),
            ];
            const display = compressSectionRows(rows);
            expect(display.map((d) => [d.label, d.indent])).toEqual([
                ["assets", 0],
                ["bank", 1],
                ["checking", 2],
                ["savings", 2],
                ["cash", 1],
            ]);
            // the compressed rows keep pointing at the engine rows that carry the amounts
            expect(display[1].row).toBe(rows[1]);
        });

        it("compresses mid-chain segments below a branching parent", () => {
            const rows = [row("assets", 0, 900), row("assets:broker", 0, 700), row("assets:broker:cash", 700, 700), row("assets:cash", 200, 200)];
            expect(compressSectionRows(rows).map((d) => [d.label, d.indent])).toEqual([
                ["assets", 0],
                ["broker:cash", 1],
                ["cash", 1],
            ]);
        });
    });

    describe("compressPeriodRows", () => {
        const prow = (account: string, values: MixedAmount[]) => ({account, depth: account.split(":").length, values});

        it("collapses a parent whose values equal its only child's in every bucket", () => {
            const rows = [prow("assets", [usd(100), usd(200)]), prow("assets:bank", [usd(100), usd(200)])];
            expect(compressPeriodRows(rows)).toEqual([{label: "assets:bank", indent: 0, row: rows[1]}]);
        });

        it("keeps a parent that differs from its only child in any bucket", () => {
            const rows = [prow("assets", [usd(100), usd(250)]), prow("assets:bank", [usd(100), usd(200)])];
            expect(compressPeriodRows(rows).map((d) => d.label)).toEqual(["assets", "bank"]);
        });
    });
});
