import {describe, expect, it} from "vitest";
import {kindLabel, localToday, pageUrl, paidText, previewUrl, standing, templateChoices, type TemplateEntry} from "./invoices.js";

describe("UNIT invoices", () => {
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
});
