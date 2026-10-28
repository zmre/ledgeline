// Settings › hledger config: the editable form and the save body it becomes.
// Pure — the panel is markup over these.
//
// A field is TEXT while it is being edited (an empty depth box means "not
// set", which a number cannot say), and a section is only sent when it
// changed, so a save touches exactly the lines the user touched.

import {CONF_SECTIONS, type ConfSection, type DepthSpec, type HledgerConf, type RegexDepth, type SaveHledgerConfBody} from "./hledgerConfTypes";

/** The deepest depth the engine writes (`conf_api::MAX_DEPTH`). */
export const MAX_CONF_DEPTH = 99;

export interface OverrideForm {
    regex: string;
    depth: string;
}

export interface SectionForm {
    /** The plain `--depth N`, as typed; "" = not set. */
    all: string;
    overrides: OverrideForm[];
}

export interface ConfForm {
    auto: boolean;
    sections: Record<ConfSection, SectionForm>;
}

/** Labels for the five sections, by what each one drives in Ledgeline. */
export const SECTION_LABELS: Record<ConfSection, string> = {
    general: "Every report (general section)",
    balancesheet: "Balance Sheet",
    incomestatement: "P&L",
    cashflow: "Cash Flow",
    balance: "Net Worth",
};

/** What a section's depth does on screen — BS/P&L have no depth control (plans 12/13), so it opens their groups instead. */
export const SECTION_EFFECT: Record<ConfSection, string> = {
    general: "Applies to every report below unless its own section sets a depth.",
    balancesheet: "Sets which groups start expanded on the Balance Sheet.",
    incomestatement: "Sets which groups start expanded on the P&L.",
    cashflow:
        "The Cash Flow tab's default depth (a depth in the URL still wins). Per-account depths are written for hledger; the tab's single depth control uses the plain one.",
    balance:
        "The Net Worth tab's default depth (hledger's `balance` command; a depth in the URL still wins). Per-account depths are written for hledger; the tab uses the plain one.",
};

function sectionForm(spec: DepthSpec): SectionForm {
    return {
        all: spec.all === null ? "" : String(spec.all),
        overrides: spec.perRegex.map((entry) => ({regex: entry.regex, depth: String(entry.depth)})),
    };
}

export function toForm(conf: HledgerConf): ConfForm {
    return {
        auto: conf.auto,
        sections: Object.fromEntries(CONF_SECTIONS.map((section) => [section, sectionForm(conf.depth[section])])) as Record<ConfSection, SectionForm>,
    };
}

export const blankOverride = (): OverrideForm => ({regex: "", depth: ""});

/** A whole number in range, or null for "" and anything else. */
function parseDepth(text: string): number | null {
    const trimmed = text.trim();
    if (!/^\d+$/.test(trimmed)) return null;
    const value = Number(trimmed);
    return value <= MAX_CONF_DEPTH ? value : null;
}

/**
 * Why a pattern cannot be a `--depth REGEX=N` value, or null — the engine's
 * `hledger_conf::regex_refusal`, mirrored so the form can say it before a round trip.
 */
export function regexProblem(regex: string): string | null {
    if (regex === "") return "is empty";
    if (/\s/.test(regex)) return "contains a space, which a config file cannot hold — use . to match one";
    if (regex.includes("#")) return "contains #, which starts a comment in a config file";
    if (regex.startsWith("-")) return "starts with -, which hledger would read as an option";
    if (regex.includes("=")) return "contains =, which separates it from the depth";
    try {
        new RegExp(regex, "i");
    } catch {
        return "is not a valid regular expression";
    }
    return null;
}

/** Every reason the form cannot be saved as it stands, one sentence each. */
export function validateForm(form: ConfForm): string[] {
    const errors: string[] = [];
    for (const section of CONF_SECTIONS) {
        const {all, overrides} = form.sections[section];
        const label = SECTION_LABELS[section];
        if (all.trim() !== "" && parseDepth(all) === null) errors.push(`${label}: the depth must be a whole number from 0 to ${MAX_CONF_DEPTH}, or empty.`);
        const seen = new Set<string>();
        for (const override of overrides) {
            const regex = override.regex.trim();
            const problem = regexProblem(regex);
            if (problem !== null) errors.push(`${label}: the account pattern ${JSON.stringify(regex)} ${problem}.`);
            if (parseDepth(override.depth) === null)
                errors.push(`${label}: the depth for ${JSON.stringify(regex)} must be a whole number from 0 to ${MAX_CONF_DEPTH}.`);
            if (seen.has(regex)) errors.push(`${label}: the account pattern ${JSON.stringify(regex)} is listed twice.`);
            seen.add(regex);
        }
    }
    return errors;
}

function specOf(form: SectionForm): {all: number | null; perRegex: RegexDepth[]} {
    return {
        all: parseDepth(form.all),
        perRegex: form.overrides.map((override) => ({regex: override.regex.trim(), depth: parseDepth(override.depth) ?? 0})),
    };
}

function sameSpec(a: DepthSpec, b: DepthSpec): boolean {
    return (
        a.all === b.all &&
        a.perRegex.length === b.perRegex.length &&
        a.perRegex.every((entry, i) => entry.regex === b.perRegex[i].regex && entry.depth === b.perRegex[i].depth)
    );
}

/** The save body for what changed between `conf` and `form` — only those keys. Call after `validateForm` is empty. */
export function toSaveBody(conf: HledgerConf, form: ConfForm): SaveHledgerConfBody {
    const body: SaveHledgerConfBody = {revision: conf.revision};
    if (form.auto !== conf.auto) body.auto = form.auto;
    for (const section of CONF_SECTIONS) {
        const wanted = specOf(form.sections[section]);
        if (!sameSpec(wanted, conf.depth[section])) body.depth = {...body.depth, [section]: wanted};
    }
    return body;
}

/** Whether saving would change anything. */
export function isDirty(conf: HledgerConf, form: ConfForm): boolean {
    const body = toSaveBody(conf, form);
    return body.auto !== undefined || body.depth !== undefined;
}
