// Human words for the file's keys (invoicer PLAN, Phase 4 feedback H1). The
// core's problem messages name frontmatter keys in backticks (`invoicee_email`
// is missing) because the CLI and `check --json` are read by people who edit
// the file; the app shows the same problems to people who use the form, so it
// says "Invoicee email is missing". The codes and keys are unchanged: this is
// presentation only.

/** The form's label for each frontmatter key the invoicer reads or writes. */
export const FIELD_LABELS: Readonly<Record<string, string>> = {
    type: "Type",
    title: "Title",
    status: "Status",
    invoice_num: "Invoice number",
    date: "Date",
    due_date: "Due date",
    terms: "Terms",
    term: "Term",
    reference: "Reference",
    po_num: "PO number",
    template: "Template",
    customer: "Customer",
    invoicee_name: "Invoicee name",
    invoicee_email: "Invoicee email",
    invoicee_phone: "Invoicee phone",
    invoicee_address: "Invoicee address",
    invoicer: "Your company",
    invoicer_name: "Your company's name",
    invoicer_email: "Your company's email",
    invoicer_phone: "Your company's phone",
    invoicer_address: "Your company's address",
    paid: "Paid",
    sent_date: "Sent date",
    sent_via: "Sent via",
    void_date: "Void date",
    deposit_for: "Deposit for note",
    emails: "Emails",
    phones: "Phones",
    addresses: "Addresses",
    company: "Company",
    revenue_account: "Revenue account",
    invoice_template: "Invoice template",
    ledger_customer: "Ledger customer",
    contracts: "Contracts",
    billing_portal: "Billing portal",
    open_pos: "Open POs",
    snippets: "Snippets",
    snippet_kind: "Snippet kind",
    aliases: "Aliases",
    tags: "Tags",
};

const KEY = /^[a-z][a-z0-9]*(?:_[a-z0-9]+)*$/;

/** A key's label: the known one, else its words (`support_tier` → "Support tier"). */
export function fieldLabel(key: string): string {
    const known = FIELD_LABELS[key];
    if (known !== undefined) return known;
    const words = key.replace(/[_-]+/g, " ").trim();
    return words === "" ? key : words.charAt(0).toUpperCase() + words.slice(1);
}

/** A label as it reads inside a sentence: "due date", but "PO number" keeps its capitals. */
function midSentence(label: string): string {
    if (/^[A-Z]{2}/.test(label)) return label;
    return label.charAt(0).toLowerCase() + label.slice(1);
}

/**
 * A problem's message for the app: each backticked key becomes its label
 * ("`invoicee_email` is missing" → "Invoicee email is missing"), a backticked
 * `key: value` becomes the label and the value in quotes, and anything else
 * backticked is quoted. The first letter is a capital.
 */
export function friendlyProblem(message: string): string {
    const out = message.replace(/`([^`]*)`/g, (_whole, inner: string, offset: number) => {
        const atStart = message.slice(0, offset).trim() === "";
        const text = inner.trim();
        const pair = /^([a-z][a-z0-9_]*):\s*(.*)$/.exec(text);
        if (KEY.test(text)) {
            const label = fieldLabel(text);
            return atStart ? label : midSentence(label);
        }
        if (pair !== null && pair[2] !== "" && !pair[2].startsWith("//")) {
            const label = fieldLabel(pair[1]);
            return `${atStart ? label : midSentence(label)} “${pair[2]}”`;
        }
        return `“${text}”`;
    });
    return out.charAt(0).toUpperCase() + out.slice(1);
}
