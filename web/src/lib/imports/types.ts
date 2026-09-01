// Domain types for CSV import rules files — what the SPA renders, decoded from
// the engine's `/api/rules*` wire by nativeDecode.ts.
//
// These mirror crates/ledgeline-server/src/rules_api.rs one field at a time, but
// they are NOT the wire: the wire omits absent settings entirely
// (`skip_serializing_if = "Option::is_none"`), and "the file does not say" is a
// fact the UI has to render differently from a value, so every optional setting
// becomes an explicit `null` here rather than an absent key.
//
// The single most important shape below is `RulesItem`. A rules document is an
// ORDERED list of items that tile the file's bytes, and the editor's whole
// contract is that every item the server sent comes back — as itself, or as
// `{kind:"keep", id}`, or in `delete`. An item this file cannot represent is an
// item the editor would silently drop, which is why `opaque`/`trivia` carry
// their raw text rather than being skipped.

/** One discovered `*.rules` file, summarized without opening it. */
export interface RulesFileSummary {
    /** Forward-slash path relative to the scan root. THIS is the handle every route takes. */
    readonly id: string;
    readonly label: string;
    /** Fingerprint over the file's raw bytes; echoed back on save for optimistic concurrency. */
    readonly revision: string;
    readonly sizeBytes: number;
    /** False ⇒ the scan never read the contents, so every summary field below is meaningless. */
    readonly parsed: boolean;
    readonly account1: string | null;
    readonly account2: string | null;
    readonly ifBlockCount: number;
    readonly editableBlockCount: number;
    readonly opaqueItemCount: number;
    readonly warnings: readonly string[];
}

/** `GET /api/rules` — every rules file beside the open journal. */
export interface RulesIndex {
    /** The scan root's final path component. The engine deliberately never sends a path. */
    readonly rootLabel: string;
    /** False ⇒ this server has no journal bound to an editor, so saving is impossible. */
    readonly editable: boolean;
    /** A scan cap was hit, so `files` is a subset. */
    readonly truncated: boolean;
    readonly files: readonly RulesFileSummary[];
    readonly warnings: readonly string[];
}

/** One resolved setting plus the item that produced it — the id is what makes a panel a view rather than a copy. */
export interface RulesPref<T> {
    readonly value: T;
    readonly itemId: number;
}

/** The `source` setting, which needs one more field than any other. */
export interface RulesSourcePref {
    /** The path or command as written. Never resolved, globbed or executed. */
    readonly value: string;
    /** hledger runs this through the shell on `import`. Ledgeline never will. */
    readonly executesShellCommand: boolean;
    readonly itemId: number;
}

/** The `fields` setting: the CSV's column names, in column order. */
export interface RulesFieldsPref {
    readonly names: readonly string[];
    readonly itemId: number;
}

/**
 * What a rules file says, flattened last-one-wins by the engine.
 *
 * `null` throughout means "the file does not say", which is NOT hledger's
 * default for that setting — choosing a default is a rendering decision.
 */
export interface RulesSettings {
    readonly source: RulesSourcePref | null;
    readonly archive: RulesPref<boolean> | null;
    readonly encoding: RulesPref<string> | null;
    readonly separator: RulesPref<string> | null;
    readonly decimalMark: RulesPref<string> | null;
    readonly dateFormat: RulesPref<string> | null;
    readonly timezone: RulesPref<string> | null;
    readonly newestFirst: RulesPref<boolean> | null;
    readonly intraDayReversed: RulesPref<boolean> | null;
    readonly skip: RulesPref<number> | null;
    readonly balanceType: RulesPref<string> | null;
    readonly account1: RulesPref<string> | null;
    readonly account2: RulesPref<string> | null;
    readonly currency: RulesPref<string> | null;
    readonly fields: RulesFieldsPref | null;
}

/** Something hledger would probably reject, anchored to where it is. Never a refusal to open the file. */
export interface RulesWarning {
    readonly itemId: number | null;
    /** 1-based line, or 0 for a warning about the file as a whole. */
    readonly line: number;
    readonly message: string;
}

/** Why the engine declined to classify an item. A closed set, spelled out in `rules_api.rs`. */
export type OpaqueReason =
    "ifTable" | "combinedMatcher" | "matchGroup" | "commentLikeMatcher" | "controlFlowInBlock" | "unparsedBlockBody" | "unparsedDirective" | "unclassified";

/** `if MATCHER` on the same line, or a bare `if` with its matchers stacked below. */
export type IfLayout = "inline" | "stacked";

/** Common to every item: its id in THIS parse, and where it sits in the file. */
interface RulesItemBase {
    /**
     * The item's 0-based index in this parse — deliberately NOT stable across
     * saves. Parse, plan and save against one document version; a stale id is
     * refused rather than guessed at.
     */
    readonly id: number;
    /** 1-based line of the item's body. */
    readonly line: number;
    /** Lines the item's whole SPAN covers, leading comments and trailing blanks included. */
    readonly lines: number;
}

export interface RulesTriviaItem extends RulesItemBase {
    readonly kind: "trivia";
    readonly text: string;
    readonly truncated: boolean;
}

export interface RulesDirectiveItem extends RulesItemBase {
    readonly kind: "directive";
    readonly name: string;
    /** Verbatim to end of line, trailing whitespace included — for `date-format` that really is part of the value. */
    readonly value: string;
}

export interface RulesIncludeItem extends RulesItemBase {
    readonly kind: "include";
    readonly target: string;
}

export interface RulesFieldsItem extends RulesItemBase {
    readonly kind: "fields";
    readonly names: readonly string[];
}

export interface RulesAssignmentItem extends RulesItemBase {
    readonly kind: "assignment";
    readonly field: string;
    readonly value: string;
}

/** A matcher of a conditional block. `field` null = a whole-record match. */
export interface RulesMatcher {
    readonly field: string | null;
    readonly pattern: string;
}

/**
 * One OR-branch of a conditional block: its matchers are AND-ed together.
 *
 * The AND is hledger's own line-prefix `&`, and the wire carries it as NESTING
 * rather than as text — no `&` ever appears in a `RulesMatcher.pattern` in
 * either direction. A plain OR list, which is every rules file this editor
 * could already open, is simply one matcher per group.
 */
export interface RulesMatcherGroup {
    readonly matchers: readonly RulesMatcher[];
}

export interface RulesAssignmentSpec {
    readonly field: string;
    readonly value: string;
}

export interface RulesIfBlockItem extends RulesItemBase {
    readonly kind: "ifBlock";
    readonly layout: IfLayout;
    /** The OR-ed groups, in file order. Always at least one, each with at least one matcher. */
    readonly groups: readonly RulesMatcherGroup[];
    readonly assignments: readonly RulesAssignmentSpec[];
}

export interface RulesOpaqueItem extends RulesItemBase {
    readonly kind: "opaque";
    readonly reason: OpaqueReason;
    /** A short sanitized preview of the first body line. */
    readonly label: string;
    readonly text: string;
    readonly truncated: boolean;
}

/** One paragraph of a rules file: the unit that can be reordered or deleted. */
export type RulesItem = RulesTriviaItem | RulesDirectiveItem | RulesIncludeItem | RulesFieldsItem | RulesAssignmentItem | RulesIfBlockItem | RulesOpaqueItem;

/** `GET /api/rules/{*id}` — one parsed rules file, item by item. */
export interface RulesDocument {
    readonly id: string;
    readonly label: string;
    /** Echo this back in a save to prove the edit is against these bytes. */
    readonly revision: string;
    readonly editable: boolean;
    /** Detected, never imposed: a CRLF file rewritten with LF shows every line as changed. */
    readonly newline: "lf" | "crlf";
    readonly settings: RulesSettings;
    readonly items: readonly RulesItem[];
    readonly warnings: readonly RulesWarning[];
}

/**
 * How the engine read one CSV column when drafting a new rules file.
 *
 * The drafted document's `fields` list already says WHAT each column became;
 * this says how sure that was, which is the half a mapping screen needs to mark
 * a guess as a guess. `field: null` is a real answer, not a missing one — the
 * engine declines to map a column it cannot claim, and the column keeps a plain
 * name so a rule can still interpolate it.
 */
export interface RulesColumnGuess {
    readonly index: number;
    readonly field: string | null;
    /** 0..=1. Orders guesses and marks the shaky ones; nothing computes with it. */
    readonly confidence: number;
}

/**
 * `POST /api/rules-create` — a starting-point rules file for a staged upload.
 *
 * `doc` is the same `RulesDocument` every other rules route returns, and
 * `preview` the same `RulesPreview`, precisely so the create screen renders
 * through the components and decoders that already exist. `doc.revision` is the
 * empty string: nothing has been written, and that value is what a follow-up
 * save carries to mean "there is no file yet".
 */
export interface RulesDraft {
    readonly doc: RulesDocument;
    readonly preview: RulesPreview;
    readonly columns: readonly RulesColumnGuess[];
    /** What the draft assumed, in sentences. Never a refusal. */
    readonly warnings: readonly string[];
}

/** Why a preview has nothing to show. On every one of these, nothing on disk was read. */
export type PreviewUnavailable = "noDataFile" | "sourceIsCommand" | "sourceOutsideRoot" | "notRegularFile" | "unreadable" | "notUtf8" | "empty";

/** `GET /api/rules-preview/{*id}` — the first few rows of the data file, so a column can be labelled with what it holds. */
export interface RulesPreview {
    readonly available: boolean;
    readonly reason: PreviewUnavailable | null;
    /** The data file's NAME only — never a path. */
    readonly dataLabel: string | null;
    readonly separator: string;
    /** The record at index `skip - 1`, when the file has `skip >= 1`. */
    readonly header: readonly string[] | null;
    readonly rows: readonly (readonly string[])[];
    readonly columns: number;
    readonly truncated: boolean;
}
