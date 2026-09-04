// Creating a rules file: the pure half.
//
// The Create screen is deliberately thin, because almost all of it already
// exists. A drafted document is a `RulesDocument` like any other, so `toForm`
// turns it into the same `FormItem[]` the Edit Rules tab edits, and
// `RowMappingPanel`/`AccountsPanel` edit it through the same `withFieldNames` /
// `withSetting` they already use. What is genuinely new is only what is here:
//
//   1. turning a draft into a SAVE request, which differs from an edit's in one
//      structural way (below);
//   2. defaulting the new file's name from the CSV the user is importing;
//   3. the two questions a draft has to answer before it is worth saving.
//
// # Why a create request cannot reuse `toSaveRequest` directly
//
// `toSaveRequest` emits `{kind:"keep", id}` for anything unchanged, which tells
// the engine to re-emit that item's ORIGINAL BYTES. A file that does not exist
// has no original bytes, so every item of a create has to be a typed body with
// no id at all. That is exactly what `toSaveRequest` produces for an item whose
// id is null — so this strips the ids and reuses it, rather than growing a
// second builder that could disagree with the first about how an `ifBlock` is
// spelled.

import type {SaveRulesBody} from "$lib/api/native";
import {fieldNames, toForm, toSaveRequest, validateForm, type FormItem, type RulesForm} from "./model";
import type {RulesDocument} from "./types";

/**
 * The `revision` that means "there is no file yet".
 *
 * The engine branches a `PUT` on this exact value, and it can never collide
 * with a real revision — those are always `LEN-HASH` in hex. Spelled once here
 * so the store and its tests agree with the engine's own constant.
 */
export const NEW_FILE_REVISION = "";

/** The `.rules` suffix every id must carry. */
const RULES_SUFFIX = ".csv.rules";

/**
 * The id to offer for a new rules file, derived from where the CSV is going.
 *
 * A rules file lives beside its data file and hledger finds it by name —
 * `bank.csv` is read through `bank.csv.rules` — so defaulting from the CSV
 * destination is not a convenience, it is the convention that makes the pair
 * work at all. The directory comes along for the same reason: a rules file in
 * the wrong directory is one hledger will not find.
 *
 * `csvPath` is the destination field's value, relative to the journal, and may
 * be anything the user has typed so far. An empty or suffix-less one falls back
 * to a plain name rather than producing something that is not a rules id.
 */
export function defaultRulesId(csvPath: string): string {
    const trimmed = csvPath.trim().replace(/^\/+/, "");
    if (trimmed === "") return "import.csv.rules";
    // Only the final `.csv` is replaced, so `2026/bank.csv` keeps its directory
    // and `bank.export.csv` keeps the part of its name that is not the suffix.
    const withoutCsv = trimmed.replace(/\.csv$/i, "");
    return `${withoutCsv}${RULES_SUFFIX}`;
}

/**
 * Is `id` shaped like something the engine will accept?
 *
 * A friendly gate in front of `validate_id`, NOT a second copy of it — the
 * engine remains the authority and its `400` is shown verbatim. What this
 * catches is the two mistakes a person actually makes in a filename field, and
 * it names them where they can be fixed.
 */
export function checkRulesId(id: string): string | null {
    const trimmed = id.trim();
    if (trimmed === "") return "Give the file a name.";
    if (!/\.rules$/i.test(trimmed)) return "The name has to end in `.rules` — that is how hledger finds it.";
    if (trimmed.startsWith("/")) return "Use a path relative to your journal, with no leading `/`.";
    if (/\\|:/.test(trimmed)) return "A name cannot contain `\\` or `:`.";
    if (trimmed.split("/").some((part) => part === "" || part === "." || part === "..")) {
        return "A name cannot contain `.` or `..`, or an empty folder name.";
    }
    if (trimmed.split("/").some((part) => part.startsWith("."))) {
        return "A hidden name (one starting with `.`) would not be listed afterwards.";
    }
    return null;
}

/** Strip an item's id so the engine reads it as an insert rather than an edit. */
function asNewItem(item: FormItem): FormItem {
    if (item.kind === "kept") {
        // Unreachable against a real draft, and asserted in the engine
        // (`every_drafted_item_can_be_written_back`): `ItemBody` has no comment
        // variant, so a draft carries no trivia and no opaque item. Throwing
        // rather than dropping, because silently omitting an item would write a
        // file that is missing a line the user was shown.
        throw new Error(`a drafted rules file cannot contain a ${item.source.kind} item`);
    }
    return {...item, id: null};
}

/**
 * The `PUT` body that writes a drafted document as a new file.
 *
 * Every item typed and id-less, nothing deleted, and the "there is no file yet"
 * revision — which is what makes the engine take its create path, resolve the
 * id against a directory rather than against a scan, and write exclusively.
 */
export function createSaveRequest(form: RulesForm): SaveRulesBody {
    const idless: RulesForm = {
        ...form,
        revision: NEW_FILE_REVISION,
        items: form.items.map(asNewItem),
    };
    // An EMPTY baseline: with nothing to diff against, every item comes out as
    // its own typed body rather than as a `keep`.
    return toSaveRequest({...idless, items: []}, idless);
}

/** A drafted document as the editable form the mapping and account panels take. */
export function draftForm(doc: RulesDocument): RulesForm {
    return toForm(doc);
}

/**
 * Why this draft is not ready to save, or null.
 *
 * Deliberately short. `validateForm` already checks everything a rules file
 * needs in general; what a NEW one needs on top is the two things only a person
 * can supply — a name, and the account this statement belongs to. Anything
 * subtler is the engine's to refuse.
 *
 * "The account this statement belongs to" has two honest spellings, and both
 * satisfy this check: a fixed value (`AccountsPanel`'s text field, a top-level
 * `account1` assignment) for a statement that is all one account, or a column
 * NAMED `account1` in `fields` (`RowMappingPanel`) for a statement — a
 * QuickBooks-style export naming a different account per row is the motivating
 * case — where it varies row by row. Checking only the first used to leave the
 * button disabled for someone who had already answered the question the
 * second way.
 */
export function createBlocker(id: string, form: RulesForm): string | null {
    const idProblem = checkRulesId(id);
    if (idProblem !== null) return idProblem;
    const account1 = form.items.find((item) => item.kind === "assignment" && item.field === "account1");
    const fixed = account1 !== undefined && account1.kind === "assignment" && account1.value.trim() !== "";
    const perRow = (fieldNames(form.items) ?? []).includes("account1");
    if (!fixed && !perRow) {
        return "Say which account this statement is for — every imported row needs one leg there.";
    }
    const errors = validateForm(form);
    return errors[0] ?? null;
}

/**
 * A one-line rendering of what the drafted file will say, item by item.
 *
 * Display only, and NOT what gets written: the bytes come from the engine's own
 * renderer when the save lands. This exists because a user about to create a
 * file in their journal directory is entitled to see what is going in it, and
 * the alternative — showing nothing until after the write — is how a generated
 * file becomes something nobody reads.
 *
 * A `kept` item renders as its own text, which a draft never has; it is handled
 * so this function is total rather than because it can happen.
 */
export function draftLines(items: readonly FormItem[]): string[] {
    return items.map((item) => {
        switch (item.kind) {
            case "directive":
                return item.value === "" ? item.name : `${item.name} ${item.value}`;
            case "fields":
                return `fields ${item.names.join(", ")}`;
            case "assignment":
                return item.value === "" ? item.field : `${item.field} ${item.value}`;
            case "ifBlock":
                return `if ${item.groups.map((group) => group.matchers.map((matcher) => matcher.pattern).join(" & ")).join(" / ")}`;
            case "kept":
                return item.source.kind === "trivia" || item.source.kind === "opaque" ? item.source.text.trimEnd() : "";
        }
    });
}
