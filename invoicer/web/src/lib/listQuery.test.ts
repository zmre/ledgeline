// The lists' URL state (invoicer PLAN 5b, S1–S4): parse and serialize round
// trip, unreadable values are dropped, the sort toggles, quick jump.
import {describe, expect, it} from "vitest";
import {
    ariaSort,
    clearInvoiceFilters,
    countText,
    customerFiltered,
    customerQueryString,
    invoiceFiltered,
    invoiceQueryString,
    isAged,
    nextSort,
    parseCustomerQuery,
    parseInvoiceQuery,
    quickJump,
    shownInvoiceSort,
    sortCustomersBy,
    sortInvoicesBy,
} from "./listQuery.js";

const invoices = (qs: string) => parseInvoiceQuery(new URLSearchParams(qs));
const customers = (qs: string) => parseCustomerQuery(new URLSearchParams(qs));

describe("UNIT invoice list URL", () => {
    it("reads every parameter of the contract and writes it back in a fixed order", () => {
        const q = invoices("dir=asc&sort=total&q=%20PO%20&aged=31-60&to=2026-10-31&from=2026-10-01&customer=Customer%20X&status=overdue");
        expect(q).toEqual({
            status: "overdue",
            customer: "Customer X",
            from: "2026-10-01",
            to: "2026-10-31",
            aged: "31-60",
            paidFrom: "",
            paidTo: "",
            q: " PO ",
            sort: "total",
            dir: "asc",
        });
        expect(invoiceQueryString(q)).toBe("status=overdue&customer=Customer+X&from=2026-10-01&to=2026-10-31&aged=31-60&q=PO&sort=total&dir=asc");
        expect(invoiceQueryString(invoices(invoiceQueryString(q)))).toBe(invoiceQueryString(q));
    });

    it("reads and writes the paid-date range (the dashboard's paid figure, R14)", () => {
        const q = invoices("paid_to=2026-11-22&paid_from=2026-10-24");
        expect([q.paidFrom, q.paidTo]).toEqual(["2026-10-24", "2026-11-22"]);
        expect(invoiceQueryString(q)).toBe("paid_from=2026-10-24&paid_to=2026-11-22");
        expect(invoiceFiltered(q)).toBe(true);
        expect(invoiceQueryString({...q, status: "paid"})).toBe("status=paid&paid_from=2026-10-24&paid_to=2026-11-22");
        expect(invoices("paid_from=last-month").paidFrom).toBe("");
        expect(clearInvoiceFilters(q)).toMatchObject({paidFrom: "", paidTo: ""});
    });

    it("drops what the server wouldn't read, so a bad link shows the whole list", () => {
        const q = invoices("status=unpaid&from=last-week&to=2026-13&aged=old&sort=amount&dir=sideways");
        expect(invoiceQueryString(q)).toBe("");
        expect(invoiceFiltered(q)).toBe(false);
        expect(invoices("sort=number&dir=nope").dir).toBe("asc");
    });

    it("knows the aging buckets and ranges", () => {
        for (const ok of ["current", "1-30", "31-60", "61-90", "91+", "90+", "0-0", "16-45", "46-"]) expect(isAged(ok), ok).toBe(true);
        for (const bad of ["", "60-31", "-5", "old", "-", "+"]) expect(isAged(bad), bad).toBe(false);
    });

    it("reads the dashboard's `N-` as `N+`, for any edges", () => {
        expect(invoices("aged=91-").aged).toBe("91+");
        expect(invoices("aged=46-").aged).toBe("46+");
        expect(invoiceQueryString(invoices("aged=8-"))).toBe("aged=8%2B");
    });

    it("writes nothing for the default list, so /invoices stays bare", () => {
        expect(invoiceQueryString(invoices(""))).toBe("");
        expect(shownInvoiceSort(invoices(""))).toEqual({sort: "date", dir: "desc"});
    });

    it("sorts by a header: a new column starts its own way, the same one flips, the default is no sort", () => {
        let q = invoices("");
        q = sortInvoicesBy(q, "total");
        expect([q.sort, q.dir]).toEqual(["total", "desc"]);
        q = sortInvoicesBy(q, "total");
        expect([q.sort, q.dir]).toEqual(["total", "asc"]);
        q = sortInvoicesBy(q, "customer");
        expect([q.sort, q.dir]).toEqual(["customer", "asc"]);
        // Date, newest first, is the server's own order: no parameters.
        q = sortInvoicesBy(invoices(""), "date");
        expect([q.sort, q.dir]).toEqual(["date", "asc"]);
        q = sortInvoicesBy(q, "date");
        expect(invoiceQueryString(q)).toBe("");
        expect(nextSort({sort: "due", dir: "asc"}, "due")).toEqual({sort: "due", dir: "desc"});
    });

    it("says how each header is sorted", () => {
        const shown = shownInvoiceSort(invoices("sort=due&dir=asc"));
        expect(ariaSort(shown, "due")).toBe("ascending");
        expect(ariaSort(shown, "date")).toBe("none");
        expect(ariaSort({sort: "date", dir: "desc"}, "date")).toBe("descending");
    });

    it("clears the filters and keeps the sort", () => {
        const q = clearInvoiceFilters(invoices("status=paid&q=x&sort=number&dir=asc"));
        expect(invoiceQueryString(q)).toBe("sort=number&dir=asc");
    });

    it("jumps to an exact number, else the only row", () => {
        const rows = [{number: "105"}, {number: "1050"}, {number: null}];
        expect(quickJump(rows, " 105 ")).toBe(rows[0]);
        expect(quickJump(rows, "10")).toBeNull();
        expect(quickJump([rows[1]], "contoso")).toBe(rows[1]);
        expect(quickJump(rows, "")).toBeNull();
        expect(countText(1, "invoice", "invoices")).toBe("1 invoice");
        expect(countText(3, "invoice", "invoices")).toBe("3 invoices");
    });
});

describe("UNIT customer list URL", () => {
    it("reads and writes the contract", () => {
        const q = customers("owes=true&overdue=1&kind=person&q=fred&sort=last-payment&dir=desc");
        expect(q).toEqual({owes: true, overdue: true, kind: "person", q: "fred", sort: "last-payment", dir: "desc"});
        expect(customerQueryString(q)).toBe("owes=true&overdue=true&kind=person&q=fred&sort=last-payment&dir=desc");
        expect(customerFiltered(q)).toBe(true);
        expect(customerQueryString(customers("kind=robot&sort=age&owes=no"))).toBe("");
    });

    it("sorts: Name A–Z is the default, money and dates start biggest and latest", () => {
        let q = sortCustomersBy(customers(""), "owed");
        expect(customerQueryString(q)).toBe("sort=owed&dir=desc");
        q = sortCustomersBy(customers(""), "name");
        expect(customerQueryString(q)).toBe("sort=name&dir=desc");
        q = sortCustomersBy(q, "name");
        expect(customerQueryString(q)).toBe("");
    });
});
