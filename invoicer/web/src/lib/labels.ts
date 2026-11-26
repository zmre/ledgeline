// Field names as people say them (invoicer PLAN, Phase 4 feedback H1 and
// J10): messages, problems and notes name a field by its label ("Invoicee
// email"), never by its frontmatter key (`invoicee_email`). The core's text,
// its codes and `check --json` keep the keys (CLI readers edit the file);
// this is presentation only. One flat map, shared by the customer, invoice,
// catalog and editor pages; add a key when a message shows it.

/** Each frontmatter key the app may mention, and what to call it. */
export const FIELD_LABELS: Readonly<Record<string, string>> = {
    // Customers.
    title: "Name",
    type: "Kind",
    kind: "Kind",
    emails: "Emails",
    phones: "Phones",
    addresses: "Addresses",
    company: "Company",
    terms: "Terms",
    term: "Term",
    revenue_account: "Revenue account",
    invoice_template: "Invoice template",
    ledger_customer: "Name in the books",
    contracts: "Contracts",
    billing_portal: "Vendor portal",
    open_pos: "Open POs",
    snippets: "Default snippets",
    body: "Notes",
    // Invoices.
    customer: "Customer",
    invoice_num: "Number",
    date: "Date",
    due_date: "Due date",
    reference: "Reference",
    po_num: "PO number",
    template: "Template",
    status: "Status",
    paid: "Paid",
    sent_date: "Sent date",
    sent_via: "Sent via",
    void_date: "Void date",
    deposit_for: "Deposit for note",
    invoicee_name: "Invoicee name",
    invoicee_email: "Invoicee email",
    invoicee_phone: "Invoicee phone",
    invoicee_address: "Invoicee address",
    invoicer: "Your company",
    invoicer_name: "Invoicer name",
    invoicer_email: "Invoicer email",
    invoicer_phone: "Invoicer phone",
    invoicer_address: "Invoicer address",
    invoicer_logo: "Invoicer logo",
    // Snippets and notes.
    snippet_kind: "Snippet kind",
    aliases: "Aliases",
    tags: "Tags",
};

const known = (key: string): boolean => Object.hasOwn(FIELD_LABELS, key);

/** What to call `key`: its label, else the key with spaces ("po_number" → "Po number"). */
export function fieldLabel(key: string): string {
    if (known(key)) return FIELD_LABELS[key];
    const words = key.replace(/[_-]+/g, " ").trim();
    return words === "" ? key : words.charAt(0).toUpperCase() + words.slice(1);
}

/** A caught error for the app: its message through `friendlyMessage`. */
export function errorText(error: unknown): string {
    return friendlyMessage(error instanceof Error ? error.message : String(error));
}

/** Several keys in a sentence: "Terms, Phones". */
export function fieldList(keys: readonly string[]): string {
    return keys.map(fieldLabel).join(", ");
}

/** A snippet's keys as its form names them; others as everywhere else. */
const SNIPPET_FIELD_LABELS: Readonly<Record<string, string>> = {title: "Title", snippet_kind: "Kind", body: "Text"};

/** A snippet's changed keys in a sentence: "Kind, Text". */
export function snippetFieldList(keys: readonly string[]): string {
    return keys.map((key) => (Object.hasOwn(SNIPPET_FIELD_LABELS, key) ? SNIPPET_FIELD_LABELS[key] : fieldLabel(key))).join(", ");
}

/** A label as it reads inside a sentence: "due date", but "PO number" keeps its capitals. */
function midSentence(label: string): string {
    return /^[A-Z]{2}/.test(label) ? label : label.charAt(0).toLowerCase() + label.slice(1);
}

/**
 * A server message or problem for the app:
 * - a backticked known key becomes its label ("`invoicee_email` is missing"
 *   → "Invoicee email is missing"; "… before `date`" → "… before date");
 * - a backticked `key: value` becomes the label and the value in quotes
 *   ("`status: sending`" → "Status “sending”");
 * - other backticked code is quoted (“invoice:”);
 * - a bare known key before a colon becomes its label ("revenue_account: …");
 * - a sentence starts with a capital (a path or a name at the start stays).
 */
export function friendlyMessage(message: string): string {
    const backticked = message.replace(/`([^`]*)`/g, (_whole, inner: string, offset: number) => {
        const atStart = message.slice(0, offset).trim() === "";
        const label = (key: string): string => (atStart ? fieldLabel(key) : midSentence(fieldLabel(key)));
        const text = inner.trim();
        const bare = /^([a-z][a-z0-9_]*):?$/.exec(text);
        if (bare !== null && known(bare[1])) return label(bare[1]);
        const pair = /^([a-z][a-z0-9_]*):\s*(.+)$/.exec(text);
        if (pair !== null && known(pair[1])) return `${label(pair[1])} “${pair[2]}”`;
        return `“${text}”`;
    });
    const out = backticked.replace(/\b([a-z]+(?:_[a-z]+)+)\b(?=:)/g, (whole: string, key: string) => (known(key) ? FIELD_LABELS[key] : whole));
    return /^[a-z][a-z']*(\s|$)/.test(out) ? out.charAt(0).toUpperCase() + out.slice(1) : out;
}

/** A setting on the Settings page, as people say it: a label and one help sentence. */
export interface SettingLabel {
    label: string;
    help: string;
}

/**
 * Every setting the Settings page shows (SETTINGS-PLAN §1), by its name in the
 * settings file (`invoices-dir`, `words.quantity`) or in the shared prefs
 * (`settlementLeadDays`), plus Ledgeline's `[books]` facts (`books.*`). The
 * names are lookup keys only: the page shows the label and the help, never a
 * name, and no help mentions one.
 */
export const SETTINGS: Readonly<Record<string, SettingLabel>> = {
    // Company.
    company: {label: "Your company", help: "The organization note whose name, address and logo go on your invoices."},
    // New invoices.
    filename: {label: "New invoice file name", help: "How a new invoice's file is named. It must include the number, so no two invoices share a file."},
    "number-start": {label: "First invoice number", help: "The number the first invoice gets. After that, each invoice gets one more than the highest so far."},
    "default-terms": {
        label: "Default payment terms",
        help: "Days until a new invoice is due, when neither the invoice nor its customer says. Leave it empty for no due date.",
    },
    "default-template": {label: "Default template", help: "The look a new invoice starts with, when its customer has no template of its own."},
    // Accounts.
    "receivable-account": {label: "Receivable account", help: "Where accrual books record what customers owe you."},
    "revenue-account": {label: "Revenue account", help: "Where a row's amount books when neither the row, its catalog item nor its customer names an account."},
    "deposit-account": {label: "Deposit account", help: "Where deposit invoices book until their final invoice uses them up."},
    "discount-account": {
        label: "Discount account",
        help: "An account for every discount that doesn't name one. Without it, a discount is shared over the rows it applies to.",
    },
    // Payments.
    "cash-deposits": {label: "Deposit payments", help: "In cash books, when a deposit counts as income."},
    "payment-allocation": {
        label: "Splitting a payment over rows",
        help: "In cash books, how a payment that doesn't cover the whole invoice is shared among its rows.",
    },
    settlementLookbackDays: {
        label: "How far back to look for payments",
        help: "How many days after an invoice a payment may arrive and still be offered as its payment. 0 looks back forever.",
    },
    settlementLeadDays: {label: "Payment before invoice", help: "How many days before an invoice's date a payment may arrive and still count for it."},
    // Editing.
    "recalc-on-save": {
        label: "Recalculate drafts after outside edits",
        help: "What happens when a draft's rows change in another editor and its totals no longer add up.",
    },
    // Dashboard.
    "aging-buckets": {label: "Aging ranges", help: "The last day of each range after Current: one to eight numbers, each larger than the one before."},
    "dso-days": {label: "Days-sales-outstanding window", help: "How many days back days-sales-outstanding is measured over."},
    "paid-window-days": {label: "Paid window", help: "The period the dashboard's Paid card starts on, in days."},
    // Files and folders.
    "invoices-dir": {label: "Invoices folder", help: "Where new invoices are written. Existing ones are found wherever they are."},
    "customers-dir": {label: "Customers folder", help: "Where new customers are written."},
    "templates-dir": {label: "Templates folder", help: "Where new templates and snippets are written, and starter templates are copied."},
    catalog: {label: "Catalog note", help: "The note new catalog items go in."},
    "pdf-dir": {label: "PDF folder", help: "Where exported PDFs are kept. Notes in it are never read as invoices."},
    // Books (Ledgeline's, read-only here).
    "books.basis": {label: "Accrual or cash", help: "Whether these books count income when you invoice it, or when you're paid. Set in Ledgeline."},
    "books.closed-through": {label: "Closed through", help: "Entries on or before this day are kept as they are. Set in Ledgeline."},
    // Wording.
    "words.description": {label: "Description column", help: "Headers that mark a table's description column."},
    "words.quantity": {label: "Quantity column", help: "Headers that mark the quantity column."},
    "words.rate": {label: "Rate column", help: "Headers that mark the rate column."},
    "words.amount": {label: "Amount column", help: "Headers that mark the amount column."},
    "words.date": {label: "Date column", help: "Headers that mark the date column."},
    "words.unit": {label: "Unit column", help: "Headers that mark the unit column."},
    "words.item": {label: "Item code column", help: "Headers that mark the column of catalog codes."},
    "words.subtotal": {label: "Subtotal rows", help: "Labels of subtotal rows."},
    "words.total": {label: "Total rows", help: "Labels of total rows; the first ends a new invoice's table."},
    "words.deposit": {label: "Deposit rows", help: "Labels of a row that applies a deposit."},
    "words.tax": {label: "Tax rows", help: "Labels of a new tax row."},
    "words.customer-tag": {label: "Customer tag", help: "The one tag that marks a contact note as a customer."},
    "words.billing-labels": {label: "Billing contact labels", help: "Which email, phone and address to bill, in order of preference."},
};

/**
 * The choices not built yet (SETTINGS-PLAN D3), shown disabled: for a setting
 * with choices, the choice itself; for one with none yet, what happens now and
 * what will be offered.
 */
export const COMING_LATER: Readonly<Record<string, {label: string; now?: string}>> = {
    "oldest-row-first": {label: "Pay the oldest row first"},
    "keep-deferral": {label: "Keep a deposit as a deferral, counted as income as its work is invoiced"},
    "first-open-day": {label: "Sending into a closed period", now: "Refused: change the date to an open day first"},
    "reverse-and-rebook": {label: "Editing a sent invoice", now: "Its booking is replaced where it stands"},
};

/** What the later choice of a setting with none yet will be. */
export const COMING_LATER_CHOICE: Readonly<Record<string, string>> = {
    "first-open-day": "Book it on the first open day",
    "reverse-and-rebook": "Reverse it and book it again",
};

/** A setting's label; one the map doesn't know, its name in words. */
export function settingLabel(key: string): string {
    return Object.hasOwn(SETTINGS, key) ? SETTINGS[key].label : fieldLabel(key.replace(/^words\./, ""));
}

/**
 * A server sentence for the Settings page, without the file's vocabulary: the
 * settings file by what it is, never `ledgeline.toml` or `[invoicer]`; a
 * setting named in backticks by its label; a file-name field by its name
 * ("the number", not `{invoice_num}`); and a link by its title.
 */
export function settingsMessage(message: string): string {
    const named = message
        .replace(/`([^`]+)`/g, (whole, inner: string) => (Object.hasOwn(SETTINGS, inner) ? midSentence(SETTINGS[inner].label) : whole))
        .replace(/\bledgeline\.toml\s*\[invoicer(?:\.words)?\]/g, "the settings file")
        .replace(/\[invoicer(?:\.words)?\]\s*(?:in\s+)?/g, "")
        .replace(/\bledgeline\.toml\b/g, "the settings file")
        .replace(/\{invoice_num\}/g, "the number")
        .replace(/\{date\}/g, "the date")
        .replace(/\{customer\}/g, "the customer")
        .replace(/\[\[([^\]]+)\]\]/g, "$1");
    return friendlyMessage(named).replace(/^the settings file/, "The settings file");
}
