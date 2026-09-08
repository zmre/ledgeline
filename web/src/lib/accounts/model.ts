// Every decision the Account List Editor (Settings → Accounts) makes, as pure
// functions. No Svelte, no DOM, no fetch — the same split `aliasModel.ts`
// states and for the same reason: the vitest project here is node-only and
// excludes `*.svelte.test.ts`, so a decision living inside a component is
// untested by construction.

import type {SaveAccountEdit, SaveAccountsBody, SaveAccountTag} from "$lib/api/native";
import type {AccountEntry, AccountFile, AccountTag} from "./types";

/** One option a closed special tag accepts. */
export interface SpecialTagOption {
    /** What gets written into the file. */
    readonly value: string;
    /** What the dropdown shows. */
    readonly label: string;
}

/**
 * One tag the app's reports read for meaning, not just carry as decoration.
 *
 * `kind: "closed"` mirrors a Rust `parse_*_tag` function's exact accepted
 * vocabulary — restated here because there is no codegen across this seam,
 * the same trade `nativeDecode.ts`'s header already makes for 28 wire
 * structs. `kind: "free"` is a tag whose value is a label the ACCOUNT chooses
 * (a balance-sheet line, an income-statement line) rather than a fact the
 * engine classifies by — offered as a plain field with suggestions, not a
 * dropdown, because there is no wrong answer to constrain it to.
 */
export interface SpecialTagSpec {
    /** The tag name, lowercase — matched against a declaration case-insensitively. */
    readonly key: string;
    readonly label: string;
    /** One or two sentences: what the tag does and which report reads it. */
    readonly help: string;
    readonly kind: "closed" | "free";
    /** For `kind: "closed"`: the complete accepted vocabulary, in display order. */
    readonly options?: readonly SpecialTagOption[];
    /** For `kind: "free"`: example values offered as a datalist, not a constraint. */
    readonly suggestions?: readonly string[];
}

/**
 * Every special tag this app's reports classify accounts by, in the order
 * they read most naturally as a form: what the account fundamentally IS
 * (`type`), which statement box and line it lands in (`issection`/`isgroup`
 * for the income statement, `bsgroup`/`bsterm` for the balance sheet), then
 * the holdings-specific pair.
 *
 * Sourced from `reports/account_types.rs::parse_account_type_tag`,
 * `reports/income_statement.rs::parse_is_section_tag`,
 * `reports/account_groups.rs::parse_bs_term_tag`,
 * `holdings/classify.rs::parse_holdings_tag` and `::parse_valuation_tag`.
 */
export const SPECIAL_TAGS: readonly SpecialTagSpec[] = [
    {
        key: "type",
        label: "Type",
        help: "The account's fundamental category. Reports fall back to guessing this from the name when it isn't set.",
        kind: "closed",
        options: [
            {value: "A", label: "Asset"},
            {value: "L", label: "Liability"},
            {value: "E", label: "Equity"},
            {value: "R", label: "Revenue"},
            {value: "X", label: "Expense"},
            {value: "C", label: "Cash"},
            {value: "V", label: "Conversion"},
            {value: "G", label: "Gain"},
        ],
    },
    {
        key: "issection",
        label: "Income statement section",
        help: "Which box of the income statement this account's activity is grouped into.",
        kind: "closed",
        options: [
            {value: "revenue", label: "Revenue"},
            {value: "cogs", label: "Cost of revenue (COGS)"},
            {value: "opex", label: "Operating expenses"},
            {value: "depreciation", label: "Depreciation & amortization"},
            {value: "interest", label: "Interest"},
            {value: "tax", label: "Taxes"},
            {value: "other", label: "Other (non-operating)"},
        ],
    },
    {
        key: "isgroup",
        label: "Income statement line",
        help: "The label this account's balance prints under, within its income-statement section — free text, your own words.",
        kind: "free",
    },
    {
        key: "bsgroup",
        label: "Balance sheet line",
        help: 'The label this account’s balance prints under on the balance sheet (e.g. "Investments").',
        kind: "free",
        suggestions: ["Cash and cash equivalents", "Accounts receivable", "Investments", "Credit cards", "Accounts payable"],
    },
    {
        key: "bsterm",
        label: "Balance sheet term",
        help: "Whether this balance-sheet line is a current (short-term) or non-current (long-term) item.",
        kind: "closed",
        options: [
            {value: "current", label: "Current"},
            {value: "noncurrent", label: "Non-current"},
        ],
    },
    {
        key: "holdings",
        label: "Holdings tab",
        help: "Which Holdings sub-tab this account's balance appears on.",
        kind: "closed",
        options: [
            {value: "stock", label: "Stocks"},
            {value: "other", label: "Other"},
            {value: "none", label: "None (exclude from Holdings)"},
        ],
    },
    {
        key: "valuation",
        label: "Valuation role",
        help: "What this account's balance represents: money actually paid in, or a valuation adjustment layered on top of it.",
        kind: "closed",
        options: [
            {value: "cost", label: "Cost (money paid in)"},
            {value: "unrealized", label: "Unrealized gain/loss"},
            {value: "depreciation", label: "Depreciation"},
            {value: "adjustment", label: "Adjustment"},
            {value: "mark", label: "Mark"},
        ],
    },
];

/** One row of the merged account list: a name, and its declaration if it has one. */
export interface AccountRow {
    readonly name: string;
    /** `null` when the account is known only from postings, never declared. */
    readonly entry: AccountEntry | null;
}

/**
 * Every account name the journal knows about, merged with its declaration.
 *
 * `allNames` is the same flat list `AccountInput`/`AccountTreeSelect` already
 * read off `journal.accountNames` — every account any posting mentions. A
 * declared account not yet mentioned by a posting (freshly added, or a plan
 * for one not yet used) is folded in from `files` so it is never lost.
 */
export function mergedRows(allNames: readonly string[], files: readonly AccountFile[]): AccountRow[] {
    const declared = new Map<string, AccountEntry>();
    for (const file of files) {
        for (const entry of file.accounts) declared.set(entry.name, entry);
    }
    const names = new Set<string>(allNames);
    for (const name of declared.keys()) names.add(name);
    return [...names].sort((a, b) => a.localeCompare(b)).map((name) => ({name, entry: declared.get(name) ?? null}));
}

/** The editable form for one account's special tags, other tags, and note. */
export interface AccountForm {
    readonly name: string;
    /** Keyed by {@link SpecialTagSpec.key}; "" means unset. Always has every key in {@link SPECIAL_TAGS}. */
    special: Record<string, string>;
    /** Every tag NOT in {@link SPECIAL_TAGS}, in file order. */
    tags: AccountTag[];
    note: string;
}

function specFor(key: string): SpecialTagSpec | undefined {
    return SPECIAL_TAGS.find((spec) => spec.key === key);
}

/** The form `entry` (or nothing, for an undeclared account) starts from. */
export function toForm(name: string, entry: AccountEntry | null): AccountForm {
    const special: Record<string, string> = {};
    for (const spec of SPECIAL_TAGS) special[spec.key] = "";
    const consumed = new Set<string>();
    const rest: AccountTag[] = [];

    for (const tag of entry?.tags ?? []) {
        const key = tag.name.toLowerCase();
        const spec = consumed.has(key) ? undefined : specFor(key);
        if (spec === undefined) {
            rest.push({...tag});
            continue;
        }
        if (spec.kind === "free") {
            special[key] = tag.value;
            consumed.add(key);
            continue;
        }
        const match = spec.options?.find((option) => option.value.toLowerCase() === tag.value.trim().toLowerCase());
        if (match === undefined) {
            // An unrecognised value for a closed tag: kept as an ordinary tag
            // row rather than silently discarded — the dropdown shows
            // "(none)", but a save that touches nothing else still keeps it.
            rest.push({...tag});
            continue;
        }
        special[key] = match.value;
        consumed.add(key);
    }

    return {name, special, tags: rest, note: entry?.note ?? ""};
}

/** `form`'s special tags (in {@link SPECIAL_TAGS} order) followed by its ordinary ones — the shape a save request sends. */
function combinedTags(form: AccountForm): AccountTag[] {
    const special: AccountTag[] = [];
    for (const spec of SPECIAL_TAGS) {
        const value = form.special[spec.key];
        if (value !== undefined && value !== "") special.push({name: spec.key, value});
    }
    const rest = form.tags.filter((tag) => tag.name.trim() !== "");
    return [...special, ...rest];
}

/** A tag-for-tag, order-sensitive comparison — two forms are equal iff a save would write nothing. */
function sameTags(a: readonly AccountTag[], b: readonly AccountTag[]): boolean {
    return a.length === b.length && a.every((tag, i) => tag.name === b[i]?.name && tag.value === b[i]?.value);
}

/** Whether `form` differs from what `entry` (or an undeclared account) holds. */
export function isDirty(entry: AccountEntry | null, form: AccountForm): boolean {
    const base = toForm(form.name, entry);
    return !sameTags(combinedTags(base), combinedTags(form)) || base.note !== form.note;
}

/**
 * Whether comma-segment `segment` would be read back as a `name:value` tag —
 * the exact predicate `ledgeline_core::parse::tag_in_segment` applies, so a
 * note that would misparse is caught here before the round trip, not just by
 * the server's own (authoritative) check.
 */
function looksLikeTag(segment: string): boolean {
    const colon = segment.indexOf(":");
    if (colon === -1) return false;
    // Rust's predicate is "does the text before the colon split into at least
    // one whitespace-delimited token" — equivalent to "is it non-blank" once
    // trimmed, since `split_whitespace` never yields an empty token.
    return segment.slice(0, colon).trim() !== "";
}

/** The first comma-segment of `note` that would misparse as a tag, or null. */
export function noteWouldMisparse(note: string): string | null {
    for (const segment of note.split(",")) {
        if (looksLikeTag(segment)) return segment.trim();
    }
    return null;
}

/** Two-or-more consecutive spaces/tabs — the run `split_account_name` reads as ending a name early. */
function containsWhitespaceRun(text: string): boolean {
    let run = false;
    for (const ch of text) {
        if (ch === " " || ch === "\t") {
            if (run) return true;
            run = true;
        } else {
            run = false;
        }
    }
    return false;
}

/** Whether `text` contains an ASCII control character (codepoint < 0x20, or DEL). */
function hasControlCharacter(text: string): boolean {
    for (const ch of text) {
        const code = ch.codePointAt(0) ?? 0;
        if (code < 0x20 || code === 0x7f) return true;
    }
    return false;
}

/** Mirrors `ledgeline_core::accounts::check_name`. `null` when `name` is fine to declare. */
export function validateName(name: string): string | null {
    if (name === "") return "An account name may not be empty.";
    if (name.trim() !== name) return "An account name may not begin or end with whitespace.";
    if (hasControlCharacter(name)) return "An account name may not contain a control character.";
    if (name.includes(";") || name.includes("#")) return 'An account name may not contain ";" or "#".';
    if (containsWhitespaceRun(name)) {
        return "An account name may not contain two consecutive spaces or tabs — hledger would read that as the end of the name.";
    }
    return null;
}

/** Client-side validation mirroring `ledgeline_core::accounts`'s own checks, so a bad value is caught before a round trip. */
export function validateForm(form: AccountForm): string[] {
    const problems: string[] = [];
    const nameProblem = validateName(form.name);
    if (nameProblem !== null) problems.push(nameProblem);

    for (const spec of SPECIAL_TAGS) {
        if (spec.kind !== "free") continue;
        const value = form.special[spec.key] ?? "";
        if (value.includes(",")) problems.push(`"${spec.label}" may not contain ",".`);
    }
    for (const tag of form.tags) {
        if (tag.name.trim() === "") continue; // a blank row is not yet a tag; silently dropped on save
        const special = specFor(tag.name.toLowerCase());
        if (special !== undefined) {
            problems.push(`"${tag.name}" is a special tag — use the "${special.label}" field above instead of an ordinary tag row.`);
            continue;
        }
        if (/\s/.test(tag.name)) problems.push(`Tag name "${tag.name}" may not contain whitespace.`);
        if (tag.name.includes(",") || tag.name.includes(":")) problems.push(`Tag name "${tag.name}" may not contain "," or ":".`);
        if (tag.value.includes(",")) problems.push(`Tag "${tag.name}"'s value may not contain ",".`);
    }
    const badSegment = form.note === "" ? null : noteWouldMisparse(form.note);
    if (badSegment !== null) problems.push(`The note "${badSegment}" would be read back as a tag — remove the comma or the colon.`);
    return problems;
}

/** Build the one edit `form` implies, against `entry` (or none, for a new declaration). */
export function toEdit(entry: AccountEntry | null, form: AccountForm): SaveAccountEdit {
    const tags: SaveAccountTag[] = combinedTags(form).map((tag) => ({name: tag.name, value: tag.value}));
    return entry === null ? {kind: "declare", name: form.name, tags, note: form.note} : {kind: "replace", index: entry.index, tags, note: form.note};
}

export function toSaveRequest(revision: string, entry: AccountEntry | null, form: AccountForm): SaveAccountsBody {
    return {revision, edits: [toEdit(entry, form)]};
}

/** A `delete` request removing one declaration. */
export function deleteSaveRequest(revision: string, index: number): SaveAccountsBody {
    return {revision, edits: [{kind: "delete", index}]};
}

/** A fresh, empty tag row for "add a tag". */
export function blankTag(): AccountTag {
    return {name: "", value: ""};
}

/**
 * The file new declarations go into, absent an explicit choice: the LAST file
 * the listing offers.
 *
 * Mirrors `budget_api::budget_lines`'s own tie-break ("ties go to the LAST one
 * the parse read, which is where a freshly `include`d file sits, because the
 * `include` is appended at EOF") — a freshly created `accounts.journal` is
 * exactly that file, so this is what makes it the default target the moment
 * it exists, with no special-casing of its name.
 */
export function defaultTargetFile(files: readonly AccountFile[]): AccountFile | null {
    return files.length === 0 ? null : (files[files.length - 1] ?? null);
}
