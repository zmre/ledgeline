// Field names as people say them (Phase 4 feedback J10): messages and
// notes name a field by its label ("Invoicee email"), never by its
// frontmatter key (`invoicee_email`). One flat map, shared by the customer
// and invoice pages; add a key when a message shows it.

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
    template: "Template",
    status: "Status",
    paid: "Paid",
    sent_date: "Sent date",
    sent_via: "Sent via",
    invoicee_name: "Invoicee name",
    invoicee_email: "Invoicee email",
    invoicee_phone: "Invoicee phone",
    invoicee_address: "Invoicee address",
    invoicer_name: "Invoicer name",
    invoicer_email: "Invoicer email",
    invoicer_phone: "Invoicer phone",
    invoicer_address: "Invoicer address",
};

/** What to call `key`: its label, else the key with spaces ("po_number" → "Po number"). */
export function fieldLabel(key: string): string {
    const known = FIELD_LABELS[key];
    if (known !== undefined) return known;
    const words = key.replace(/[_-]+/g, " ").trim();
    return words === "" ? key : words.charAt(0).toUpperCase() + words.slice(1);
}

/** Several keys in a sentence: "Terms, Phones". */
export function fieldList(keys: readonly string[]): string {
    return keys.map(fieldLabel).join(", ");
}

/**
 * A server message with each known key replaced by its label: a key in
 * backticks (`` `invoicee_email` is missing `` → "Invoicee email is
 * missing") or bare at the start of the message or before a colon
 * ("revenue_account: …"). Other text is left as it is.
 */
export function friendlyMessage(message: string): string {
    // A sentence starts with a capital; a path or a name at the start stays as written.
    const capital = (s: string): string => (/^[a-z][a-z']*(\s|$)/.test(s) ? s.charAt(0).toUpperCase() + s.slice(1) : s);
    const out = message
        .replace(/`([A-Za-z_][\w-]*):?`/g, (whole: string, key: string) => (key in FIELD_LABELS ? FIELD_LABELS[key] : whole))
        .replace(/\b([a-z]+(?:_[a-z]+)+)\b(?=:)/g, (whole: string, key: string) => (key in FIELD_LABELS ? FIELD_LABELS[key] : whole));
    return capital(out);
}
