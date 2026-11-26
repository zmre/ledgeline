// The Settings page's "These books" form, as pure functions (SETTINGS-PLAN
// §3, §4): the sections in screen order, the form as read and as edited, what
// Save sends (only what changed; "Use the default" removes a setting), the
// new invoice file name as Date / Customer / Number chips, and the number
// fields' parsing.
//
// Settings are keyed as the file names them (`invoices-dir`, `words.total`);
// the shared payment-matching ones as the shared prefs do
// (`settlementLeadDays`). The page shows only their labels (`SETTINGS`).
import {SETTING_KEYS, WORD_KEYS, type SettingName, type SettingsPage, type SettingValue, type SettingsValues} from "./settings.js";
import {MATCHING_KEYS, MATCHING_SPEC, type Matching, type MatchingKey} from "./matching.js";

/** The word lists, as their settings are named (`words.<name>`). */
export const WORD_SETTINGS: readonly SettingName[] = WORD_KEYS.map((k) => `words.${k}` as const);

export type SectionId = "company" | "new-invoices" | "accounts" | "payments" | "editing" | "dashboard" | "files" | "books" | "wording";

export interface Section {
    id: SectionId;
    title: string;
    /** The settings in it, in order (the `[books]` facts are read-only). */
    keys: readonly string[];
}

/** The sections, in screen order. */
export const SECTIONS: readonly Section[] = [
    {id: "company", title: "Company", keys: ["company"]},
    {id: "new-invoices", title: "New invoices", keys: ["filename", "number-start", "default-terms", "default-template"]},
    {id: "accounts", title: "Accounts", keys: ["receivable-account", "revenue-account", "deposit-account", "discount-account"]},
    {id: "payments", title: "Payments", keys: ["cash-deposits", "payment-allocation", ...MATCHING_KEYS]},
    {id: "editing", title: "Editing", keys: ["recalc-on-save"]},
    {id: "dashboard", title: "Dashboard", keys: ["aging-buckets", "dso-days", "paid-window-days"]},
    {id: "files", title: "Files and folders", keys: ["invoices-dir", "customers-dir", "templates-dir", "catalog", "pdf-dir"]},
    {id: "books", title: "Books", keys: ["books.basis", "books.closed-through"]},
    {id: "wording", title: "Wording", keys: WORD_SETTINGS},
];

/** Every setting the form edits in the settings file. */
export const FILE_KEYS: readonly SettingName[] = [...SETTING_KEYS, ...WORD_SETTINGS];

/** The settings that only cash books use. */
export const CASH_ONLY_KEYS: readonly string[] = ["cash-deposits", "payment-allocation"];

/** The accounts the books find for you when unset ("Found for you: …"). */
export const INFERRED_KEYS = ["receivable-account", "revenue-account", "deposit-account"] as const;

export function isMatchingKey(key: string): key is MatchingKey {
    return (MATCHING_KEYS as readonly string[]).includes(key);
}

/** The settings with their own choices, in words; `coming` adds the disabled ones. */
export const CHOICES: Readonly<Record<string, readonly {value: string; label: string}[]>> = {
    "cash-deposits": [
        {value: "income-on-receipt", label: "Count a deposit as income when it arrives"},
        {value: "liability-until-final", label: "Hold a deposit as owed until its final invoice"},
    ],
    "payment-allocation": [
        {value: "match-rows", label: "Move each account toward its row's amount"},
        {value: "pro-rata", label: "Split each payment by the rows' proportions"},
    ],
    "recalc-on-save": [
        {value: "drafts", label: "Recalculate a draft's totals"},
        {value: "never", label: "Leave them, and show the difference"},
    ],
};

/** A value or default as the form holds it: `words` flattened to `words.<name>`. */
export type Values = Readonly<Record<string, unknown>>;

/** The page's nested values, flat: `{words: {total: […]}}` → `{"words.total": […]}`. */
export function flatten(values: SettingsValues | Record<string, unknown>): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(values as Record<string, unknown>)) {
        if (key === "words" && typeof value === "object" && value !== null && !Array.isArray(value)) {
            for (const [word, list] of Object.entries(value)) out[`words.${word}`] = list;
        } else out[key] = value;
    }
    return out;
}

/** The form: every value as edited, the settings set back to their default, and the matching settings edited. */
export interface Form {
    values: Values;
    /** "Use the default" chosen: Save removes these from the file. */
    reset: readonly SettingName[];
    /** The shared matching settings changed here (the rest show as saved). */
    matching: Partial<Matching>;
}

/** What the form started from: the settings file as read. */
export interface Baseline {
    values: Values;
    defaults: Values;
    /** The settings the file sets. */
    set: readonly string[];
}

export function baselineOf(page: SettingsPage): Baseline {
    return {values: flatten(page.values), defaults: flatten(page.defaults), set: page.set};
}

export function formOf(base: Baseline): Form {
    return {values: {...base.values}, reset: [], matching: {}};
}

/** Two values the same (lists and numbers alike). */
export const same = (a: unknown, b: unknown): boolean => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** A matching setting as the form shows it: as edited, else as saved. */
export const matchingValue = (form: Form, saved: Matching, key: MatchingKey): number => form.matching[key] ?? saved[key];

/** `key` set to `value` (and no longer reset). */
export function withValue(form: Form, key: string, value: unknown): Form {
    if (isMatchingKey(key)) return {...form, matching: {...form.matching, [key]: value as number}};
    return {...form, values: {...form.values, [key]: value}, reset: form.reset.filter((k) => k !== key)};
}

/** "Use the default": the default shown, and the setting removed on Save (a matching one: its default value saved). */
export function withDefault(form: Form, base: Baseline, key: string): Form {
    if (isMatchingKey(key)) return withValue(form, key, MATCHING_SPEC[key].fallback);
    const name = key as SettingName;
    return {...form, values: {...form.values, [name]: base.defaults[name] ?? null}, reset: [...form.reset.filter((k) => k !== name), name]};
}

/** What Save sends: the settings file's changes, and the shared matching ones. */
export interface Changes {
    set: Partial<Record<SettingName, SettingValue>>;
    unset: SettingName[];
    matching: Partial<Matching>;
}

export function changesOf(base: Baseline, form: Form, saved: Matching): Changes {
    const set: Partial<Record<SettingName, SettingValue>> = {};
    const unset: SettingName[] = [];
    for (const key of FILE_KEYS) {
        const value = form.values[key];
        // An emptied optional setting (no company, no default terms) is the default: none.
        if (form.reset.includes(key) || value === null || value === undefined) {
            if (base.set.includes(key)) unset.push(key);
        } else if (!same(value, base.values[key])) set[key] = value as SettingValue;
    }
    const matching: Partial<Matching> = {};
    for (const key of MATCHING_KEYS) {
        const value = form.matching[key];
        if (value !== undefined && value !== saved[key]) matching[key] = value;
    }
    return {set, unset, matching};
}

/** The settings with unsaved changes. */
export function changedKeys(changes: Changes): Set<string> {
    return new Set<string>([...Object.keys(changes.set), ...changes.unset, ...Object.keys(changes.matching)]);
}

export const isEmpty = (changes: Changes): boolean => changedKeys(changes).size === 0;

/** Whether the row shows "Default": the file doesn't set it (or it is reset or emptied), and it isn't changed. */
export function showsDefault(base: Baseline, form: Form, key: string, saved: Matching): boolean {
    if (isMatchingKey(key)) return matchingValue(form, saved, key) === MATCHING_SPEC[key].fallback;
    const name = key as SettingName;
    if (form.reset.includes(name)) return true;
    const value = form.values[name];
    if ((value === null || value === undefined) && (base.defaults[name] ?? null) === null) return true;
    return !base.set.includes(name) && same(value, base.values[name]);
}

/** Whether the row offers "Use the default": it doesn't show the default now. */
export const offersDefault = (base: Baseline, form: Form, key: string, saved: Matching): boolean => !showsDefault(base, form, key, saved);

// The new invoice file name.

/** A piece of the file name: typed text, or a field filled in for each invoice. */
export type NamePart = {kind: "text"; text: string} | {kind: "field"; field: string};

/** The fields a file name may hold, as the chips name them. */
export const NAME_FIELDS: readonly {field: string; label: string}[] = [
    {field: "date", label: "Date"},
    {field: "customer", label: "Customer"},
    {field: "invoice_num", label: "Number"},
];

export const nameFieldLabel = (field: string): string => NAME_FIELDS.find((f) => f.field === field)?.label ?? field;

/** The file name's extension, kept as it is (the editor edits the name before it). */
export function splitExtension(pattern: string): {stem: string; extension: string} {
    return pattern.toLowerCase().endsWith(".md") ? {stem: pattern.slice(0, -3), extension: pattern.slice(-3)} : {stem: pattern, extension: ""};
}

/** A file name's parts; text around and between fields is kept, empty included. */
export function nameParts(stem: string): NamePart[] {
    const parts: NamePart[] = [];
    const pattern = /\{([a-z_]+)\}/g;
    let at = 0;
    for (const match of stem.matchAll(pattern)) {
        parts.push({kind: "text", text: stem.slice(at, match.index)});
        parts.push({kind: "field", field: match[1]});
        at = match.index + match[0].length;
    }
    parts.push({kind: "text", text: stem.slice(at)});
    return parts;
}

/** The parts back as the setting holds them. */
export function nameOf(parts: readonly NamePart[], extension: string): string {
    return parts.map((p) => (p.kind === "text" ? p.text : `{${p.field}}`)).join("") + extension;
}

/** The parts with `field` added at the end. */
export function withField(parts: readonly NamePart[], field: string): NamePart[] {
    const last = parts[parts.length - 1];
    const lead: NamePart[] = last?.kind === "text" && last.text !== "" && !last.text.endsWith(" ") ? [{kind: "text", text: `${last.text} `}] : [];
    const kept = lead.length > 0 ? parts.slice(0, -1) : [...parts];
    return [...kept, ...lead, {kind: "field", field}, {kind: "text", text: ""}];
}

/** The parts without the field at `at` (the text either side joined). */
export function withoutField(parts: readonly NamePart[], at: number): NamePart[] {
    const before = parts[at - 1];
    const after = parts[at + 1];
    if (parts[at]?.kind !== "field" || before?.kind !== "text" || after?.kind !== "text") return [...parts];
    return [...parts.slice(0, at - 1), {kind: "text", text: before.text + after.text}, ...parts.slice(at + 2)];
}

/** What a new invoice's file would be called: "2026-11-25 Acme Invoice 1001.md". */
export function nameExample(parts: readonly NamePart[], extension: string, sample: {date: string; customer: string; number: string}): string {
    const fill: Record<string, string> = {date: sample.date, customer: sample.customer, invoice_num: sample.number};
    return parts.map((p) => (p.kind === "text" ? p.text : (fill[p.field] ?? nameFieldLabel(p.field)))).join("") + extension;
}

// Numbers.

/** A whole number from what was typed; `null` for an empty field; `undefined` when it isn't one. */
export function wholeNumber(text: string): number | null | undefined {
    const trimmed = text.trim();
    if (trimmed === "") return null;
    return /^\d+$/.test(trimmed) ? Number(trimmed) : undefined;
}

/** The aging ranges from "30, 60, 90"; `undefined` when they don't read. */
export function agingBuckets(text: string): number[] | undefined {
    const parts = text
        .split(/[,\s]+/)
        .map((p) => p.trim())
        .filter((p) => p !== "");
    if (parts.length === 0 || !parts.every((p) => /^\d+$/.test(p))) return undefined;
    return parts.map(Number);
}

/** Problems the page can see before asking the server, by setting, in words. */
export function localProblems(form: Form): Record<string, string> {
    const problems: Record<string, string> = {};
    const v = form.values;
    const buckets = v["aging-buckets"];
    if (
        !Array.isArray(buckets) ||
        buckets.length < 1 ||
        buckets.length > 8 ||
        buckets.some((b, i) => typeof b !== "number" || (i > 0 && b <= buckets[i - 1]))
    ) {
        problems["aging-buckets"] = "Give one to eight whole numbers, each larger than the one before.";
    }
    for (const key of ["dso-days", "paid-window-days"]) {
        const n = v[key];
        if (typeof n !== "number" || !Number.isInteger(n) || n < 1 || n > 3660) problems[key] = "Give a whole number of days from 1 to 3660.";
    }
    const terms = v["default-terms"];
    if (terms !== null && terms !== undefined && (typeof terms !== "number" || !Number.isInteger(terms) || terms < 0 || terms > 3650)) {
        problems["default-terms"] = "Give a whole number of days from 0 to 3650, or leave it empty.";
    }
    const filename = v.filename;
    if (typeof filename === "string" && !filename.includes("{invoice_num}"))
        problems.filename = "The file name must include the Number, so no two invoices share a file.";
    for (const key of ["invoices-dir", "customers-dir", "templates-dir", "catalog", "pdf-dir"]) {
        if (typeof v[key] === "string" && (v[key] as string).trim() === "") problems[key] = "Give a folder inside the books folder.";
    }
    for (const key of WORD_SETTINGS) {
        const list = v[key];
        if (Array.isArray(list) && list.length === 0) problems[key] = "Keep at least one word.";
        if (typeof list === "string" && list.trim() === "") problems[key] = "Give one word.";
    }
    return problems;
}
