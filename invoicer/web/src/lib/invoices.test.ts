import {describe, expect, it} from "vitest";
import {
    invoiceCounts,
    invoiceCountsTitle,
    kindLabel,
    nextNumberReason,
    localToday,
    mismatches,
    pageUrl,
    paidText,
    previewUrl,
    settlementText,
    standing,
    templateChoices,
    templateName,
    type TemplateEntry,
} from "./invoices.js";

describe("UNIT invoices", () => {
    it("says where the next number comes from (H19)", () => {
        expect(nextNumberReason("2004", "books")).toBe("next after 2004, booked in the journal");
        expect(nextNumberReason("108", "document")).toBe("next after 108");
        expect(nextNumberReason(null, "start")).toBe("the first number");
        expect(nextNumberReason(undefined, undefined)).toBeNull();
        expect(nextNumberReason("108", "elsewhere")).toBeNull();
    });

    it("splits the sidebar's invoice counts: overdue, and open but not yet due (H17)", () => {
        const counts = invoiceCounts({all: 9, drafts: 1, open: 3, overdue: 2});
        expect(counts).toEqual({overdue: 2, notDue: 1, drafts: 1});
        expect(invoiceCountsTitle({overdue: 2, notDue: 1, drafts: 1})).toBe("2 overdue · 1 open, not yet due · 1 draft, not counted");
        expect(invoiceCountsTitle({overdue: 0, notDue: 4, drafts: 0})).toBe("0 overdue · 4 open, not yet due");
        expect(invoiceCounts(undefined)).toBeNull();
    });

    it("names a note's MBR page: no extension, each segment encoded, a trailing slash", () => {
        expect(pageUrl("invoices/2026-11-10 Customer X Invoice 105.md")).toBe("/invoices/2026-11-10%20Customer%20X%20Invoice%20105/");
        expect(pageUrl("/a/b#c?.markdown")).toBe("/a/b%23c%3F/");
        expect(pageUrl("x.MD")).toBe("/x/");
    });

    it("names the preview page for a saved invoice through another template", () => {
        expect(previewUrl("105", "/invoices/tpl/classic.html")).toBe("/.ledgeline-invoicer/preview?number=105&template=%2Finvoices%2Ftpl%2Fclassic.html");
        expect(previewUrl("A&B", "/t q.html")).toBe("/.ledgeline-invoicer/preview?number=A%26B&template=%2Ft+q.html");
    });

    it("reads today in the local zone, late in the evening too", () => {
        // 23:30 in Denver is already the next day in UTC.
        expect(localToday(new Date(2026, 10, 17, 23, 30))).toBe("2026-11-17");
        expect(localToday(new Date(2026, 0, 5, 0, 1))).toBe("2026-01-05");
    });

    it("says where a document stands, overdue before open", () => {
        const sent = {status: "sent", paid: null, open: true, overdue: false} as const;
        expect(standing({...sent, status: "draft"})).toEqual({label: "Draft", tone: "neutral"});
        expect(standing({...sent, status: "void"})).toEqual({label: "Void", tone: "neutral"});
        expect(standing(sent)).toEqual({label: "Open", tone: "warning"});
        expect(standing({...sent, overdue: true})).toEqual({label: "Overdue", tone: "error"});
        expect(standing({...sent, open: false})).toEqual({label: "Paid", tone: "success"});
        expect(standing({...sent, paid: {state: "partial", parts: [{date: "2026-11-05", amount: "$1,000"}]}}).label).toBe("Partly paid");
        expect(standing({...sent, status: null}).label).toBe("No status");
    });

    it("reads the paid mirror in words", () => {
        expect(paidText(null)).toBe("—");
        expect(paidText({state: "no"})).toBe("No");
        expect(paidText({state: "full", date: "2026-10-05"})).toBe("Paid 2026-10-05");
        expect(
            paidText({
                state: "partial",
                parts: [
                    {date: "2026-10-28", amount: "$1,500"},
                    {date: "2026-11-19", amount: "$3,500"},
                ],
            })
        ).toBe("Partly: 2026-10-28 $1,500, 2026-11-19 $3,500");
    });

    it("says where an invoice stands in one plain line", () => {
        const base = {total: "$3,000.00", paid: "$0.00", outstanding: "$3,000.00", paid_on: null, due_date: "2026-11-14"};
        expect(settlementText({...base, state: "paid", paid: "$3,000.00", outstanding: "$0.00", paid_on: "2026-10-20"})).toBe("Paid in full on 2026-10-20");
        expect(settlementText({...base, state: "paid", paid_on: null})).toBe("Paid in full");
        expect(settlementText({...base, state: "partial", paid: "$1,000.00", outstanding: "$2,000.00"})).toBe(
            "Partly paid: $1,000.00 of $3,000.00, $2,000.00 due 2026-11-14"
        );
        expect(settlementText({...base, state: "unpaid"})).toBe("Unpaid: $3,000.00 due 2026-11-14");
        expect(settlementText({...base, state: "unpaid", due_date: null})).toBe("Unpaid: $3,000.00 due");
        expect(settlementText({...base, state: "void"})).toBe("Void: nothing is owed");
    });

    it("labels only the kinds that aren't invoices", () => {
        expect(kindLabel("invoice")).toBeNull();
        expect(kindLabel(null)).toBeNull();
        expect(kindLabel("estimate")).toBe("Estimate");
        expect(kindLabel("credit-note")).toBe("Credit note");
    });

    it("offers the invoice's own template first, then the rest by name", () => {
        const t = (path: string, name: string, installed = true): TemplateEntry => ({path, name, kinds: ["invoice"], starter: !installed, installed});
        const all = [t("/tpl/z.html", "Zeta"), t("/tpl/modern.html", "Modern", false), t("/tpl/classic.html", "Classic", false)];
        expect(templateChoices(all, "/tpl/z.html").map((x) => x.name)).toEqual(["Zeta", "Classic", "Modern"]);
        expect(templateChoices(all, null).map((x) => x.name)).toEqual(["Classic", "Modern", "Zeta"]);
        expect(templateChoices(all, "/tpl/missing.html")).toHaveLength(3);
    });

    it("labels the amounts that don't match their rows, totals by their word", () => {
        const table = {
            total: "$14,400.00",
            rows: [
                {row: 1, kind: "item", description: "Platform subscription, 1 year", rate: "$12,000"},
                {row: 2, kind: "item", description: "Senior architect hours", rate: "$800"},
                {row: 3, kind: "total", description: "", rate: "Total due"},
            ],
            discrepancies: [
                {row: 2, line: 33, written: "$1,600.00", expected: "$2,400.00"},
                {row: 3, line: 34, written: "**$13,600.00**", expected: "$14,400.00"},
                {row: 9, line: 40, written: "", expected: "$1.00"},
            ],
        };
        expect(mismatches(table)).toEqual([
            {row: 2, label: "Senior architect hours", written: "$1,600.00", computed: "$2,400.00"},
            {row: 3, label: "Total due", written: "**$13,600.00**", computed: "$14,400.00"},
            {row: 9, label: "Row 9", written: "(blank)", computed: "$1.00"},
        ]);
        expect(mismatches({...table, discrepancies: []})).toEqual([]);
        expect(mismatches(null)).toEqual([]);
    });

    it("names the template an invoice uses, or says it isn't in the books", () => {
        const all: TemplateEntry[] = [
            {path: "/invoices/tpl/myco-default.html", name: "MyCo default", kinds: ["invoice"], starter: false, installed: true},
            {path: "/invoices/tpl/modern.html", name: "Modern", kinds: ["invoice"], starter: true, installed: false},
        ];
        expect(templateName(all, "/invoices/tpl/myco-default.html")).toEqual({name: "MyCo default", known: true});
        expect(templateName(all, "invoices/tpl/modern.html")).toEqual({name: "Modern", known: true});
        expect(templateName(all, "/invoices/tpl/gone.html")).toEqual({name: "/invoices/tpl/gone.html", known: false});
        expect(templateName(all, null)).toBeNull();
    });
});
