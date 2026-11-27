// The sidebar's sections (ARCHITECTURE §6.3, plan 46), built from the current
// path, the open books and the unbilled count. The root layout resolves the
// hrefs and renders the result with Sidebar.svelte; this is the part a test
// can call without SvelteKit's router.
//
//   Home · Invoices · To bill · Time · Customers
//   Library ▸ Catalog, Snippets, Templates, Time accounts   (collapsible)
//   Settings                                                 (the foot)
//
// Counts (Phase 4 feedback J11, H17) say what they count. Invoices shows
// overdue (red) and open but not yet due, exclusive, when more than none,
// drafts in the tooltip only; To bill, the customers with unbilled time.
// Time is one link, current on its Overview, Entries and Track; its
// accounts are the Library's.
import {invoiceCounts, invoiceCountsTitle, type BooksInfo} from "#lib/invoices.js";

/** One count beside a section ("2 overdue"). */
export interface SidebarCount {
    /** Its testid's suffix (`sidebar-<link>-<id>`). */
    id: string;
    value: number;
    /** What it counts, after the number ("overdue", "customers"). */
    label: string;
    /** `error`: a red badge; else quiet text. */
    tone?: "error";
}

/** A section in the sidebar, its link already resolved. */
export interface SidebarLink {
    id: string;
    href: string;
    label: string;
    active: boolean;
    /** Its counts, in order; none shown when empty or absent. */
    counts?: SidebarCount[];
    /** The link's tooltip (every count in words). */
    title?: string;
    /** Its name in the narrow top bar, where a plain group's heading isn't shown. */
    narrowLabel?: string;
    /** Its accessible name when the visible label leans on a group's heading. */
    ariaLabel?: string;
}

/** A titled run of links after the main ones; collapsible when it has `onToggle` (the Library). */
export interface SidebarGroup {
    id: string;
    title: string;
    links: SidebarLink[];
    /** Collapsible only: whether it is open, as kept. */
    open?: boolean;
    /** Collapsible only: the user opened (true) or closed (false) it. */
    onToggle?: (open: boolean) => void;
}

/** Every section's resolved path. */
export interface NavHrefs {
    home: string;
    invoices: string;
    toBill: string;
    time: string;
    timeAccounts: string;
    customers: string;
    catalog: string;
    snippets: string;
    templates: string;
}

export interface NavInput {
    /** The current pathname. */
    path: string;
    hrefs: NavHrefs;
    books: BooksInfo | null;
    /** Customers with unbilled time (plan 46 B-unbilled); null before time is set up. */
    unbilledCustomers: number | null;
}

const plural = (n: number, one: string, many: string): string => (n === 1 ? one : many);

/** `href` itself or a page under it. */
const under = (path: string, href: string): boolean => {
    const base = href.replace(/\/$/, "");
    return path === href || path === base || path.startsWith(`${base}/`);
};

const total = (value: number | undefined, one: string, many: string): SidebarCount[] =>
    value === undefined ? [] : [{id: "count", value, label: plural(value, one, many)}];

/** The main sections, in order: Home, Invoices, To bill, Time, Customers. */
export function sectionLinks({path, hrefs, books, unbilledCustomers}: NavInput): SidebarLink[] {
    const invoices = invoiceCounts(books?.invoices);
    const invoiceBadges: SidebarCount[] =
        invoices === null
            ? []
            : [
                  {id: "overdue", value: invoices.overdue, label: "overdue", tone: "error" as const},
                  {id: "count", value: invoices.notDue, label: "open, not yet due"},
              ].filter((c) => c.value > 0);
    const toBill: SidebarCount[] =
        unbilledCustomers === null || unbilledCustomers === 0 ? [] : total(unbilledCustomers, "customer with unbilled time", "customers with unbilled time");
    return [
        // The dashboard is home (PLAN 5a, R1): current only on the root itself.
        {id: "home", href: hrefs.home, label: "Home", active: path.replace(/\/$/, "") === hrefs.home.replace(/\/$/, "")},
        {
            id: "invoices",
            href: hrefs.invoices,
            label: "Invoices",
            active: under(path, hrefs.invoices),
            counts: invoiceBadges,
            title: invoices === null ? undefined : invoiceCountsTitle(invoices),
        },
        {id: "to-bill", href: hrefs.toBill, label: "To bill", active: under(path, hrefs.toBill), counts: toBill},
        {id: "time", href: hrefs.time, label: "Time", active: under(path, hrefs.time) && !under(path, hrefs.timeAccounts)},
        {
            id: "customers",
            href: hrefs.customers,
            label: "Customers",
            active: under(path, hrefs.customers),
            counts: total(books?.customers, "customer", "customers"),
        },
    ];
}

/** The Library: what invoices are made from, and time's accounts. Collapsible; `open` and `onToggle` are the layout's. */
export function libraryGroup({path, hrefs, books}: NavInput, open: boolean, onToggle: (open: boolean) => void): SidebarGroup {
    const link = (id: string, href: string, label: string, counts: SidebarCount[] = []): SidebarLink => ({
        id,
        href,
        label,
        active: under(path, href),
        counts,
    });
    return {
        id: "library",
        title: "Library",
        open,
        onToggle,
        links: [
            link("catalog", hrefs.catalog, "Catalog", total(books?.catalog, "item", "items")),
            link("snippets", hrefs.snippets, "Snippets", total(books?.snippets, "snippet", "snippets")),
            link("templates", hrefs.templates, "Templates", total(books?.templates, "template", "templates")),
            link("time-accounts", hrefs.timeAccounts, "Time accounts"),
        ],
    };
}
