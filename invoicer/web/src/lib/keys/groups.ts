// The invoicer's help-sheet sections, in the order the `?` sheet shows them
// (PLAN 5k, Y4): the app-wide keys, then the sidebar's order of pages, each
// page's editor after its list. The keymap engine is packages/ui's
// (`#ui/keys/`), shared with Ledgeline; the sections are the app's own.

export const INVOICER_KEY_GROUPS = [
    "Global",
    "Go to",
    "Time",
    "Home",
    "Invoices",
    "Invoice",
    "Invoice editor",
    "Bill table",
    "To bill",
    "Customers",
    "Customer",
    "Catalog",
    "Snippets",
    "Templates",
    "Template",
    "Track",
    "Settings",
    // Time tracking (plan 46).
    "Time entries",
    "Time accounts",
] as const;

export type InvoicerKeyGroup = (typeof INVOICER_KEY_GROUPS)[number];
