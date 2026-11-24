// The display-only arithmetic against the Rust goldens
// (invoicer-core tests/goldens.rs, ARCHITECTURE §3.2 and §3.3, the fixture's 108).
import {describe, expect, it} from "vitest";
import {computeTable, formatMoney, parseMoney, parsePercent, round, type Role} from "./arithmetic";

const ROLES: Role[] = ["description", "quantity", "rate", "amount", "books"];
const money = (rows: string[][], roles: Role[] = ROLES) => {
    const t = computeTable(roles, rows);
    return {amounts: t.rows.map((r) => (r.cents === null ? null : formatMoney(r.cents))), total: formatMoney(t.total), table: t};
};

describe("arithmetic (display only), mirroring the Rust goldens", () => {
    it("the §3.3 worked example", () => {
        const {amounts, total, table} = money([
            ["Website redesign", "1", "$12,000", "$12,000.00", "revenue:projects"],
            ["Senior architect hours", "10", "$250", "", "revenue:consulting"],
            ["Loyalty discount", "", "-10%", "", ""],
            ["Annual support plan", "1", "$6,000", "", "liabilities:deferred-revenue → revenue:support, 12 months from 2026-12-01"],
            ["", "", "", "", ""],
            ["", "", "**Subtotal**", "", ""],
            ["Early-payment discount", "", "-2%", "", ""],
            ["", "", "**Subtotal**", "", ""],
            ["Sales tax", "", "8.25%", "", "liabilities:sales-tax"],
            ["Less deposit (Invoice 104)", "", "", "-$5,000.00", "liabilities:customer-deposits"],
            ["", "", "**Total due**", "", ""],
        ]);
        expect(amounts).toEqual([
            "$12,000.00",
            "$2,500.00",
            "-$250.00",
            "$6,000.00",
            null,
            "$20,250.00",
            "-$405.00",
            "$19,845.00",
            "$1,637.21",
            "-$5,000.00",
            "$16,482.21",
        ]);
        expect(total).toBe("$16,482.21");
        expect(table.rows.map((r) => r.kind)).toEqual([
            "item",
            "item",
            "adjustment",
            "item",
            "spacer",
            "total",
            "adjustment",
            "total",
            "adjustment",
            "fixed-adjustment",
            "total",
        ]);
        // Each adjustment's base: the nearest item or total above.
        expect(table.rows[2].appliesTo).toBe(1);
        expect(table.rows[6].appliesTo).toBe(5);
        expect(table.rows[8].appliesTo).toBe(7);
    });

    it("the §3.2 invoice (105): per-item discounts, a subtotal, the deposit", () => {
        const {amounts, total} = money([
            ["Senior architect hours", "10", "$800", "$8,000", "revenue:consulting"],
            ["Friends and family discount", "", "-10%", "-$800", ""],
            ["Junior engineer", "50", "$200", "$10,000", "revenue:consulting:delivery"],
            ["Friends and family discount", "", "-5%", "-$500", ""],
            ["", "", "**Subtotal**", "**$16,700**", ""],
            ["Less deposit (Invoice 104)", "", "", "-$8,350", "liabilities:customer-deposits"],
            ["", "", "**Total due**", "**$8,350**", ""],
        ]);
        expect(amounts).toEqual(["$8,000.00", "-$800.00", "$10,000.00", "-$500.00", "$16,700.00", "-$8,350.00", "$8,350.00"]);
        expect(total).toBe("$8,350.00");
    });

    it("the fixture's 108, with Code and Unit columns", () => {
        const roles: Role[] = ["item", "description", "quantity", "unit", "rate", "amount", "books"];
        const {amounts} = money(
            [
                [
                    "SAAS-Y",
                    "Platform subscription, 1 year",
                    "1",
                    "ea",
                    "$12,000",
                    "$12,000.00",
                    "liabilities:deferred-revenue → revenue:saas, 12 months from 2026-12-01",
                ],
                ["ARCH", "Senior architect hours", "2", "h", "$800", "$1,600.00", "revenue:consulting"],
                ["", "", "", "", "**Total due**", "**$13,600.00**", ""],
            ],
            roles
        );
        expect(amounts).toEqual(["$12,000.00", "$1,600.00", "$13,600.00"]);
    });

    it("every row kind (goldens.rs every_row_kind_golden)", () => {
        const {amounts, table} = money([
            ["Widget", "3", "$10.50", "", "revenue:goods"],
            ["Discount", "", "-10%", "", ""],
            ["Shipping", "", "", "$12", "revenue:shipping"],
            ["Credit", "", "", "-$5.00", ""],
            ["&nbsp;", "", "", "", ""],
            ["", "", "**Total**", "", ""],
        ]);
        expect(table.rows.map((r) => r.kind)).toEqual(["item", "adjustment", "item", "fixed-adjustment", "spacer", "total"]);
        expect(amounts).toEqual(["$31.50", "-$3.15", "$12.00", "-$5.00", null, "$35.35"]);
    });

    it("tax below a subtotal taxes after discounts; above it, only the item above", () => {
        const below = money([
            ["A", "1", "$100", "", ""],
            ["B", "1", "$200", "", ""],
            ["Discount", "", "-10%", "", ""],
            ["", "", "**Subtotal**", "", ""],
            ["Sales tax", "", "10%", "", ""],
            ["", "", "**Total due**", "", ""],
        ]);
        expect(below.amounts).toEqual(["$100.00", "$200.00", "-$20.00", "$280.00", "$28.00", "$308.00"]);
        const above = money([
            ["A", "1", "$100", "", ""],
            ["B", "1", "$200", "", ""],
            ["Sales tax", "", "10%", "", ""],
            ["Discount", "", "-10%", "", ""],
            ["", "", "**Total due**", "", ""],
        ]);
        expect(above.amounts).toEqual(["$100.00", "$200.00", "$20.00", "-$20.00", "$300.00"]);
    });

    it("column order doesn't change it; tax rounds half away from zero", () => {
        const {amounts} = money([
            ["Hours", "10", "$250", "", "revenue:x"],
            ["Discount", "", "-10%", "", ""],
            ["", "", "**Subtotal**", "", ""],
            ["Tax", "", "8.25%", "", "liabilities:tax"],
            ["", "", "**Total due**", "", ""],
        ]);
        expect(amounts).toEqual(["$2,500.00", "-$250.00", "$2,250.00", "$185.63", "$2,435.63"]);
    });

    it("both negative spellings are fixed adjustments; percent signs are optional", () => {
        for (const credit of ["$-800", "-$800", "- $800", "−$800"]) {
            const {table} = money([
                ["Work", "", "", "$1,000", ""],
                ["Credit", "", "", credit, ""],
            ]);
            expect(table.rows[1].kind).toBe("fixed-adjustment");
            expect(table.total).toBe(20000n);
        }
        expect(parsePercent("8.25")).toEqual(parsePercent("8.25%"));
        expect(parseMoney("$12")?.explicitSign).toBe(false);
    });

    it("rounds half away from zero, never through floats", () => {
        expect(round({m: 5n, s: 3})).toBe(1n); // 0.005 → 0.01
        expect(round({m: -5n, s: 3})).toBe(-1n);
        expect(round({m: 4n, s: 3})).toBe(0n);
        expect(formatMoney(-163721n)).toBe("-$1,637.21");
        expect(formatMoney(5n, "EUR")).toBe("0.05 EUR");
    });
});
