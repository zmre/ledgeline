// The sidebar's sections: Home, Invoices, To bill, Time, Customers in that
// order, Time current on its Overview, Entries and Track but not its
// accounts (the Library's), and the counts each says it counts.
import {describe, expect, it, vi} from "vitest";
import {libraryGroup, sectionLinks, type NavHrefs, type NavInput} from "./nav.js";

const hrefs: NavHrefs = {
    home: "/app/",
    invoices: "/app/invoices",
    toBill: "/app/to-bill",
    time: "/app/time",
    timeAccounts: "/app/time/accounts",
    customers: "/app/customers",
    catalog: "/app/catalog",
    snippets: "/app/snippets",
    templates: "/app/templates",
};
const books = {
    journal: "books.journal",
    title: "Acme Books",
    basis: "accrual" as const,
    documents: 8,
    invoices: {all: 8, drafts: 1, open: 3, overdue: 1},
    customers: 6,
    catalog: 4,
    snippets: 1,
    templates: 2,
    nextNumber: "2005",
};
const at = (path: string, over: Partial<NavInput> = {}): NavInput => ({path, hrefs, books, unbilledCustomers: 3, ...over});
const current = (input: NavInput): string[] =>
    [...sectionLinks(input), ...libraryGroup(input, false, () => undefined).links].filter((l) => l.active).map((l) => l.id);

describe("UNIT the sidebar's sections", () => {
    it("lists Home, Invoices, To bill, Time and Customers, with their counts", () => {
        const links = sectionLinks(at("/app/"));
        expect(links.map((l) => [l.id, l.label, l.href])).toEqual([
            ["home", "Home", "/app/"],
            ["invoices", "Invoices", "/app/invoices"],
            ["to-bill", "To bill", "/app/to-bill"],
            ["time", "Time", "/app/time"],
            ["customers", "Customers", "/app/customers"],
        ]);
        expect(links[1].counts).toEqual([
            {id: "overdue", value: 1, label: "overdue", tone: "error"},
            {id: "count", value: 2, label: "open, not yet due"},
        ]);
        expect(links[1].title).toBe("1 overdue · 2 open, not yet due · 1 draft, not counted");
        expect(links[2].counts).toEqual([{id: "count", value: 3, label: "customers with unbilled time"}]);
        expect(links[4].counts).toEqual([{id: "count", value: 6, label: "customers"}]);
    });

    it("counts To bill only once time is set up and something is unbilled", () => {
        expect(sectionLinks(at("/app/", {unbilledCustomers: null}))[2].counts).toEqual([]);
        expect(sectionLinks(at("/app/", {unbilledCustomers: 0}))[2].counts).toEqual([]);
        expect(sectionLinks(at("/app/", {unbilledCustomers: 1}))[2].counts?.[0].label).toBe("customer with unbilled time");
    });

    it("marks the current section: Time on its Overview, Entries and Track, its accounts in the Library", () => {
        expect(current(at("/app/"))).toEqual(["home"]);
        expect(current(at("/app/invoices/102/edit"))).toEqual(["invoices"]);
        expect(current(at("/app/to-bill"))).toEqual(["to-bill"]);
        expect(current(at("/app/time"))).toEqual(["time"]);
        expect(current(at("/app/time/entries"))).toEqual(["time"]);
        expect(current(at("/app/time/track"))).toEqual(["time"]);
        expect(current(at("/app/time/accounts"))).toEqual(["time-accounts"]);
        expect(current(at("/app/customers/new"))).toEqual(["customers"]);
        expect(current(at("/app/catalog"))).toEqual(["catalog"]);
        expect(current(at("/app/templates/Plain"))).toEqual(["templates"]);
        expect(current(at("/app/settings"))).toEqual([]);
    });

    it("puts Catalog, Snippets, Templates and Time accounts in the Library, as kept", () => {
        const onToggle = vi.fn();
        const library = libraryGroup(at("/app/"), true, onToggle);
        expect([library.id, library.title, library.open]).toEqual(["library", "Library", true]);
        expect(library.links.map((l) => [l.id, l.label, l.href, l.counts?.map((c) => `${c.value} ${c.label}`)])).toEqual([
            ["catalog", "Catalog", "/app/catalog", ["4 items"]],
            ["snippets", "Snippets", "/app/snippets", ["1 snippet"]],
            ["templates", "Templates", "/app/templates", ["2 templates"]],
            ["time-accounts", "Time accounts", "/app/time/accounts", []],
        ]);
        library.onToggle?.(false);
        expect(onToggle).toHaveBeenCalledWith(false);
        expect(libraryGroup(at("/app/", {books: null}), false, onToggle).links[0].counts).toEqual([]);
    });
});
