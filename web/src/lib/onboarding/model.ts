// File → New wizard (plan 39 Phase 6): the pure half — presets, the step
// sequence, validation and the request body. No Svelte, no fetch, so all of it
// is unit-tested under node. The engine re-checks everything it is sent; the
// checks here exist to say what is wrong before a round trip, and mirror the
// engine's (`ledgeline_core::new_ledger`, `accounts::check_name`).

import type {CreateLedgerBody, CurrencyStyle, LedgerKind, SourceApp, TemplateAccount, WizardPath, WizardStep} from "./types";

/** The longest ledger name the engine accepts, in characters. */
export const MAX_TITLE_CHARS = 120;

/** A labelled currency style the details step offers. */
export type CurrencyPreset = Readonly<{id: string; label: string; style: CurrencyStyle}>;

const style = (commodity: string, side: "left" | "right", decimals = 2, decimalMark: "period" | "comma" = "period"): CurrencyStyle =>
    Object.freeze({commodity, side, spaced: side === "right", decimals, decimalMark});

/** The currencies offered by name. "Other" (`custom`) takes any code. */
export const CURRENCY_PRESETS: readonly CurrencyPreset[] = Object.freeze([
    {id: "usd", label: "US dollar", style: style("$", "left")},
    {id: "eur", label: "Euro", style: style("€", "right", 2, "comma")},
    {id: "gbp", label: "Pound sterling", style: style("£", "left")},
    {id: "cad", label: "Canadian dollar", style: style("CAD", "right")},
    {id: "aud", label: "Australian dollar", style: style("AUD", "right")},
    {id: "nzd", label: "New Zealand dollar", style: style("NZD", "right")},
    {id: "chf", label: "Swiss franc", style: style("CHF", "right")},
    {id: "jpy", label: "Japanese yen", style: style("¥", "left", 0)},
    {id: "inr", label: "Indian rupee", style: style("₹", "left")},
]);

export const CUSTOM_CURRENCY = "custom";

export const MONTHS: readonly string[] = Object.freeze([
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
]);

export const KIND_LABELS: Readonly<Record<LedgerKind, {title: string; blurb: string}>> = Object.freeze({
    individual: {title: "Personal", blurb: "Bank accounts, cards, loans, income and everyday spending."},
    business: {title: "Business", blurb: "Receivables, payables, sales tax, owner's equity and common business expenses."},
    empty: {title: "Empty", blurb: "No accounts. You add your own, or an import brings them in."},
});

/**
 * The apps the "Start from another app" path names, and the export each one
 * gives you. Which formats this engine can read TODAY is the import screen's
 * call (`/api/import/capabilities`); the wizard only warns when it can't.
 */
export const SOURCE_APPS: readonly SourceApp[] = Object.freeze([
    {id: "banktivity", label: "Banktivity", format: "QIF", extensions: ["qif"], howTo: "Export all accounts as QIF (File → Export)."},
    {id: "quicken", label: "Quicken", format: "QIF or QMTF", extensions: ["qif", "qmtf"], howTo: "Windows: File → File Export → QIF. Mac: export a QMTF file."},
    {id: "moneydance", label: "Moneydance", format: "QIF", extensions: ["qif"], howTo: "Export your data as QIF (File → Export)."},
    {
        id: "gnucash",
        label: "GnuCash",
        format: "multi-line CSV or QIF",
        extensions: ["csv", "qif"],
        howTo: "File → Export → Export Transactions to CSV, with Simple Layout turned off.",
    },
    {id: "xero", label: "Xero", format: "Journal Report CSV", extensions: ["csv"], howTo: "Run the Journal Report over every year you want and export it."},
    {
        id: "quickbooks",
        label: "QuickBooks Online",
        format: "Journal report (Excel)",
        extensions: ["xlsx"],
        howTo: "Reports → Journal → Export → Export to Excel.",
    },
    {id: "hledger-csv", label: "hledger", format: "print -O csv", extensions: ["csv"], howTo: "Run: hledger print -O csv > books.csv"},
    {id: "ynab", label: "YNAB", format: "Register CSV", extensions: ["csv"], howTo: "Export your plan, unzip it, and choose the Register file."},
    {id: "monarch", label: "Monarch", format: "transactions CSV", extensions: ["csv"], howTo: "Download your transactions as CSV from Monarch's settings."},
    {id: "copilot", label: "Copilot", format: "transactions CSV", extensions: ["csv"], howTo: "Export your transactions as CSV from Copilot's settings."},
    {
        id: "bank",
        label: "A bank or card statement",
        format: "OFX, QFX, QBO or CSV",
        extensions: ["ofx", "qfx", "qbo", "csv"],
        howTo: "Download a statement from your bank's website.",
    },
]);

/** The steps a run visits, in order. An empty chart has nothing to review. */
export function stepsFor(path: WizardPath | null, kind: LedgerKind): readonly WizardStep[] {
    if (path === "import") return ["start", "source", "details", "create"];
    if (path === "fresh") return kind === "empty" ? ["start", "kind", "details", "create"] : ["start", "kind", "details", "accounts", "create"];
    return ["start"];
}

/** The step after `step` on this run, or null at the end. */
export function nextStep(steps: readonly WizardStep[], step: WizardStep): WizardStep | null {
    const at = steps.indexOf(step);
    return at < 0 || at + 1 >= steps.length ? null : steps[at + 1];
}

/** The step before `step` on this run, or null at the start. */
export function previousStep(steps: readonly WizardStep[], step: WizardStep): WizardStep | null {
    const at = steps.indexOf(step);
    return at <= 0 ? null : steps[at - 1];
}

/** One thousand as the new ledger will show it — `$1,000.00`, `1.000,00 €`, `¥1,000`. */
export function sampleAmount(currency: CurrencyStyle): string {
    const [group, mark] = currency.decimalMark === "comma" ? [".", ","] : [",", "."];
    const number = currency.decimals === 0 ? `1${group}000` : `1${group}000${mark}${"0".repeat(currency.decimals)}`;
    const gap = currency.spaced ? " " : "";
    return currency.side === "left" ? `${currency.commodity}${gap}${number}` : `${number}${gap}${currency.commodity}`;
}

const CURRENCY_SYMBOLS = "$€£¥₹₩₽₪₫₱₦₴₸₺₼₾฿¢₡₲₵";

/** Why `commodity` can't be the base currency, or null when it can. */
export function commodityProblem(commodity: string): string | null {
    const chars = [...commodity];
    if (chars.length === 0) return "Enter a currency code or symbol.";
    if (chars.length > 8) return "Use at most 8 characters.";
    if (!chars.every((ch) => /\p{L}/u.test(ch) || CURRENCY_SYMBOLS.includes(ch))) return "Use letters (such as USD) or one currency symbol (such as $).";
    return null;
}

/** Why `title` can't name the ledger, or null when it can. */
export function titleProblem(title: string): string | null {
    const trimmed = title.trim();
    if (trimmed === "") return "Give the ledger a name.";
    if ([...trimmed].length > MAX_TITLE_CHARS) return `Use at most ${MAX_TITLE_CHARS} characters.`;
    // eslint-disable-next-line no-control-regex -- refusing control characters is the point
    if (/[\u0000-\u001f\u007f]/.test(trimmed)) return "Use a single line of text.";
    return null;
}

/** Why `name` can't be an account, or null when it can. The engine's rules, said early. */
export function accountNameProblem(name: string): string | null {
    if (name === "") return "An account needs a name.";
    if (name !== name.trim()) return "Remove the space at the start or end.";
    if (/[;#]/.test(name)) return "Account names can't contain ; or #.";
    if (/\s\s/.test(name)) return "Two spaces in a row would end the name early.";
    // eslint-disable-next-line no-control-regex -- refusing control characters is the point
    if (/[\u0000-\u001f\u007f]/.test(name)) return "Use a single line of text.";
    return null;
}

/** The account's own `type:` tag, or null when it inherits one. */
export function declaredType(account: TemplateAccount): string | null {
    return account.tags.find((tag) => tag.name === "type")?.value ?? null;
}

const TYPE_NAMES: Readonly<Record<string, string>> = Object.freeze({
    A: "Asset",
    L: "Liability",
    E: "Equity",
    R: "Revenue",
    X: "Expense",
    C: "Cash",
    V: "Conversion",
    G: "Gain",
});

/** The effective type's name: the account's own `type:`, else its nearest listed parent's. */
export function effectiveTypeName(accounts: readonly TemplateAccount[], account: TemplateAccount): string | null {
    const byName = new Map(accounts.map((entry) => [entry.name, entry]));
    for (let name: string | null = account.name; name !== null; name = parentOf(name)) {
        const entry = byName.get(name);
        const type = entry === undefined ? null : declaredType(entry);
        if (type !== null) return TYPE_NAMES[type] ?? type;
    }
    return null;
}

function parentOf(name: string): string | null {
    const cut = name.lastIndexOf(":");
    return cut < 0 ? null : name.slice(0, cut);
}

/** The tags other than `type:`, as they will read in the file. */
export function tagSummary(account: TemplateAccount): string {
    return account.tags
        .filter((tag) => tag.name !== "type")
        .map((tag) => (tag.value === "" ? `${tag.name}:` : `${tag.name}: ${tag.value}`))
        .join(", ");
}

/** The list with the account at `index` renamed. */
export function renameAccount(accounts: readonly TemplateAccount[], index: number, name: string): readonly TemplateAccount[] {
    return accounts.map((account, at) => (at === index ? {...account, name} : account));
}

/** The list without the account at `index`. */
export function removeAccount(accounts: readonly TemplateAccount[], index: number): readonly TemplateAccount[] {
    return accounts.filter((_, at) => at !== index);
}

/** Every problem in the list, keyed by index: bad names and repeats. */
export function accountProblems(accounts: readonly TemplateAccount[]): ReadonlyMap<number, string> {
    const problems = new Map<number, string>();
    const seen = new Set<string>();
    accounts.forEach((account, index) => {
        const problem = accountNameProblem(account.name) ?? (seen.has(account.name) ? "This account is already in the list." : null);
        if (problem !== null) problems.set(index, problem);
        seen.add(account.name);
    });
    return problems;
}

/** The file extension of `name`, lowercased, without the dot; "" when none. */
export function extensionOf(name: string): string {
    const dot = name.lastIndexOf(".");
    return dot <= 0 ? "" : name.slice(dot + 1).toLowerCase();
}

/**
 * What to say about `fileName` for `app`, given the formats the import screen
 * reads today (`null` while unknown). Null when there is nothing to say.
 */
export function sourceFileWarning(app: SourceApp, fileName: string, readable: readonly string[] | null): string | null {
    const ext = extensionOf(fileName);
    if (!app.extensions.includes(ext)) {
        return `${app.label} exports ${app.extensions.map((e) => `.${e}`).join(" or ")} files; this one is ${ext === "" ? "something else" : `.${ext}`}.`;
    }
    if (readable !== null && !readable.includes(ext)) {
        return `This version of Ledgeline can't import .${ext} files yet. The ledger is still created, and you can import the file once it can.`;
    }
    return null;
}

/** A starting name for the ledger, from the folder it goes in. */
export function defaultTitle(folderName: string | null, kind: LedgerKind): string {
    if (folderName !== null && folderName.trim() !== "") return folderName.trim();
    return kind === "business" ? "Business books" : "My books";
}

/** The answers, as `POST /api/new-ledger` wants them. */
export function createBody(answers: {
    folderId: string;
    title: string;
    currency: CurrencyStyle;
    fiscalYearStart: number;
    accounts: readonly TemplateAccount[];
}): CreateLedgerBody {
    return Object.freeze({
        folderId: answers.folderId,
        title: answers.title.trim(),
        currency: answers.currency,
        fiscalYearStart: answers.fiscalYearStart,
        accounts: answers.accounts,
    });
}
