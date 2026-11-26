// The Settings page's wire (settings plan §5): `GET /api/settings`,
// `POST /api/settings/check` and `PUT /api/settings`. The Rust side is
// `crates/ledgeline-invoicer/src/app/settings_wire.rs`; its golden is
// `crates/ledgeline-invoicer/tests/snapshots/settings-wire.json` (one of each
// shape, errors included). Change all three together.
//
// Envelope fields are camelCase. Setting names are spelled as in
// `ledgeline.toml [invoicer]` (`invoices-dir`, `words.quantity`): map keys and
// list entries to look labels up by (lib/labels.ts), never text to show.
//
// The three payment-matching settings are not here: they are UI preferences
// shared with Ledgeline (`settlementLookbackDays`, `settlementToleranceBps`,
// `settlementLeadDays`), read and saved through `GET`/`PATCH /api/ui-prefs`
// and kept for these books alone through `POST /api/ui-prefs/pin`.
//
// Errors use the one envelope every /api route answers with,
// `{error, code, ...detail}`:
//
// | Status | `code`        | When                                       | Detail                 |
// |--------|---------------|--------------------------------------------|------------------------|
// | 400    | `bad-request` | an unknown setting or a value of no shape  |                        |
// | 409    | `changed`     | the revision isn't the file's any more     | `revision` (current)   |
// | 409    | `no-file`     | no `ledgeline.toml`, and no `createFile`   |                        |
// | 409    | `unusable`    | symlink, over 64 KiB, not TOML, or invalid | `problems` if invalid  |
// | 409    | `confirm`     | an effect not in `confirmed`               | `effects: Effect[]`    |
// | 422    | `invalid`     | problems with the values                   | `problems`             |
// | 422    | `refused`     | D5: deposits wait for their final invoice  | `refused: Refusal`     |
//
// Every other route answers `409` with `problem: string` too while the books
// couldn't open (a problem in this file, say); `GET /api/settings` still
// answers then, with `booksError`.

/** Settings in `[invoicer]`, as the file names them (top-level only). */
export const SETTING_KEYS = [
    "company",
    "invoices-dir",
    "customers-dir",
    "templates-dir",
    "catalog",
    "pdf-dir",
    "filename",
    "number-start",
    "default-terms",
    "default-template",
    "receivable-account",
    "revenue-account",
    "deposit-account",
    "discount-account",
    "cash-deposits",
    "payment-allocation",
    "recalc-on-save",
    "aging-buckets",
    "dso-days",
    "paid-window-days",
] as const;
export type SettingKey = (typeof SETTING_KEYS)[number];

/** `[invoicer.words]`'s settings; on the wire `words.<key>`. */
export const WORD_KEYS = [
    "description",
    "quantity",
    "rate",
    "amount",
    "date",
    "unit",
    "item",
    "subtotal",
    "total",
    "deposit",
    "tax",
    "customer-tag",
    "billing-labels",
] as const;
export type WordKey = (typeof WORD_KEYS)[number];

/** Any setting `set`, `unset`, `problems` and `effects` may name. */
export type SettingName = SettingKey | `words.${WordKey}`;

/** `[invoicer.words]`: every list's first word is the one the app writes. */
export type WordValues = Record<Exclude<WordKey, "customer-tag">, string[]> & {"customer-tag": string};

export type CashDeposits = "income-on-receipt" | "liability-until-final";
export type PaymentAllocation = "match-rows" | "pro-rata";
export type RecalcOnSave = "drafts" | "never";

/** Every setting's value (effective, or the defaults). */
export interface SettingsValues {
    company: string | null;
    "invoices-dir": string;
    "customers-dir": string;
    "templates-dir": string;
    catalog: string;
    "pdf-dir": string;
    /** With `{date}`, `{customer}` and `{invoice_num}`; show it as chips. */
    filename: string;
    /** An integer or text in the file; always text here. */
    "number-start": string | null;
    /** Net days, 0 to 3650; none by default. */
    "default-terms": number | null;
    /** A template's `template:` value (`/invoices/tpl/classic.html`). */
    "default-template": string | null;
    "receivable-account": string | null;
    "revenue-account": string | null;
    "deposit-account": string | null;
    "discount-account": string | null;
    "cash-deposits": CashDeposits;
    "payment-allocation": PaymentAllocation;
    "recalc-on-save": RecalcOnSave;
    /** 1 to 8 increasing day counts above zero. */
    "aging-buckets": number[];
    "dso-days": number;
    /** 1 to 3660. */
    "paid-window-days": number;
    words: WordValues;
}

/** A problem with one setting, or (`key` null) with the file. */
export interface SettingProblem {
    key: SettingName | null;
    /** A plain sentence; it never names a key. */
    message: string;
}

/** What a booking-related change does; send `key` back in `confirmed`. */
export interface Effect {
    key: SettingKey;
    message: string;
}

/** A deposit received in cash books that no finished final invoice used up. */
export interface WaitingDeposit {
    number: string;
    customer: string;
    date: string;
    /** In the books' money style (`$1,500.00`). */
    waiting: string;
    /** Where its payments actually booked. */
    bookedTo: string[];
}

/** A change refused whatever is confirmed (D5). */
export interface Refusal {
    key: SettingKey;
    message: string;
    deposits: WaitingDeposit[];
}

export interface TemplateChoice {
    /** What `default-template` holds. */
    value: string;
    name: string;
    kinds: string[];
    /** A starter not copied in yet. */
    starter: boolean;
}

export interface CompanyChoice {
    title: string;
    /** What `company` holds: `[[Title]]`. */
    link: string;
}

/** A choice not built yet: show it disabled, as "Coming later". */
export interface ComingLater {
    id: "oldest-row-first" | "keep-deferral" | "first-open-day" | "reverse-and-rebook";
    setting: SettingKey | null;
    section: "payments" | "editing";
    label: string;
}

/** `GET /api/settings`. */
export interface SettingsPage {
    file: "missing" | "present" | "unusable";
    /** Why the page is read-only, in plain words; null when it can save. */
    problem: string | null;
    /** The problems in `[invoicer]` as it is on disk. */
    problems: SettingProblem[];
    readOnly: boolean;
    /** Send it back with every check and save; null when the file is unusable. */
    revision: string | null;
    values: SettingsValues;
    defaults: SettingsValues;
    /** The settings the file sets: the rows that offer "Use the default". */
    set: SettingName[];
    /** "Found for you: …" for an unset account; null when none is found. */
    inferred: {
        "receivable-account": string | null;
        "revenue-account": string | null;
        "deposit-account": string | null;
    };
    nextNumber: {number: string; after: string | null; source: "document" | "books" | "start"} | null;
    /** Ledgeline's `[books]`, read-only; null fields when the books aren't open. */
    books: {basis: "cash" | "accrual" | null; closedThrough: string | null};
    /** Why the books couldn't open; null when they are open. */
    booksError: string | null;
    choices: {templates: TemplateChoice[]; companies: CompanyChoice[]; accounts: string[]};
    /** Settings Save asks to confirm ("Affects new bookings"). */
    affectsBookings: SettingKey[];
    coming: ComingLater[];
}

/** A value to set: what `SettingsValues` holds for that setting. */
export type SettingValue = string | number | string[] | number[];

/** `POST /api/settings/check` and `PUT /api/settings`: only what changed. */
export interface SettingsChange {
    revision: string | null;
    set?: Partial<Record<SettingName, SettingValue>>;
    /** "Use the default". */
    unset?: SettingName[];
    /** After `409 no-file`, once the user agreed. */
    createFile?: boolean;
    /** The effects' keys the user confirmed. */
    confirmed?: SettingKey[];
}

/** `POST /api/settings/check`'s answer. Writes nothing. */
export interface SettingsCheck {
    problems: SettingProblem[];
    effects: Effect[];
    refused: Refusal | null;
}

/** `PUT /api/settings`'s `200`. */
export interface SettingsSaved {
    revision: string;
    values: SettingsValues;
    set: SettingName[];
    /** `ledgeline.toml` was created. */
    created: boolean;
}
