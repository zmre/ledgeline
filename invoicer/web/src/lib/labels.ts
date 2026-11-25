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

/** Several keys in a sentence: "Terms, Phones". */
export function fieldList(keys: readonly string[]): string {
    return keys.map(fieldLabel).join(", ");
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
