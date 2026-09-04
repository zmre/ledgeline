// Domain types for the New Transactions flow (WP-11 lane E), decoded from the
// engine's `/api/import/*` and `/api/prefs` wire by nativeDecode.ts.
//
// They mirror "The lane E wire contract" in plans/11-enhanced-import.md field by
// field, with the same two departures `imports/types.ts` already makes from the
// rules wire:
//
//   1. An OMITTED key becomes an explicit `null`. "The format did not volunteer
//      a statement balance" and "there is no balance" are the same fact to the
//      UI and it must render it as one; a key that is sometimes missing is not.
//   2. Nothing here carries a path in the filesystem sense. `journalId`,
//      `csvPath` and a candidate `id` are all relative to the include root,
//      exactly as a rules `id` is, and the engine never sends an absolute one.
//
// No Svelte/DOM imports: these are read by `importModel.ts`, which is the pure
// node-tested half of this screen.

/** Why hledger cannot be used. `timedOut` is the amendment `HledgerError` gained during lane A. */
export type HledgerUnavailableReason = "notFound" | "tooOld" | "unrunnable" | "timedOut";

/**
 * Whether the engine can run hledger at all, and what to say when it cannot.
 *
 * `message` is the engine's own sentence and is what the banner shows; `reason`
 * only picks the remedy (a path control for `notFound`, an upgrade note for
 * `tooOld`). An unrecognised reason decodes to `null` rather than throwing —
 * losing the whole screen over a reason string we do not know would hide the one
 * message that tells the user what to fix.
 */
export interface HledgerStatus {
    readonly available: boolean;
    /** hledger's own `--version` display ("1.52"), or null when it could not be run. */
    readonly version: string | null;
    readonly reason: HledgerUnavailableReason | null;
    readonly message: string | null;
}

/** One journal file an import may be written into. Ranked by the engine; `label` decides nothing. */
export interface JournalTarget {
    /** Path relative to the include root, forward slashes. THIS is the handle every route takes. */
    readonly id: string;
    readonly label: string;
    readonly txnCount: number;
    /** ISO `YYYY-MM-DD`, or null for a file holding no transactions (an `account`/`P` directives file). */
    readonly lastTxnDate: string | null;
    readonly isRoot: boolean;
    /** False ⇒ a symlink, a non-regular file, or outside the include root. Offered but not selectable. */
    readonly writable: boolean;
}

/** Whether the engine found a git repo around the targets, and whether it will commit into it. */
export interface GitCapability {
    readonly available: boolean;
    readonly autocommit: boolean;
}

/** Why an alias is not handed to hledger for an import. */
export type AliasRefusal = "scoped" | "empty" | "control" | "tooLong" | "limit" | "stale";

/** Why an alias line is presented read-only. */
export type AliasLock = "commentLike" | "empty" | "delimiter" | "control" | "tooLong";

/**
 * One `alias` directive in the journal.
 *
 * Two verdicts, and they are genuinely independent — which is why they are two
 * pairs of fields rather than one. `forwarded` is "will an import use this";
 * `editable` is "will the GUI rewrite this line". An alias with a `;` in it is
 * forwarded but not editable (hledger reads the `;` as part of the account name,
 * so we show the line rather than cement that reading); one closed by
 * `end aliases` is editable but not forwarded.
 */
export interface AliasEntry {
    /** The file it is declared in, relative to the include root. */
    readonly journalId: string;
    /** 0-based position among that FILE's alias lines — the handle a save names. */
    readonly index: number;
    /** 1-based line number in that file. */
    readonly line: number;
    /** The pattern as written, without a regex's slashes. */
    readonly pattern: string;
    readonly replacement: string;
    /** Whether the pattern is the `/REGEX/` form. */
    readonly regex: boolean;
    readonly forwarded: boolean;
    readonly refusal: AliasRefusal | null;
    readonly refusalMessage: string | null;
    readonly editable: boolean;
    readonly lock: AliasLock | null;
    readonly lockMessage: string | null;
}

/** One journal file's alias lines, and the revision a save must echo. */
export interface AliasFile {
    readonly journalId: string;
    readonly label: string;
    /** Echo this back in a save to prove the edit is against these bytes. */
    readonly revision: string;
    readonly writable: boolean;
    readonly aliases: readonly AliasEntry[];
}

/** `GET /api/aliases` — every alias the open journal declares. */
export interface AliasListing {
    readonly editable: boolean;
    readonly files: readonly AliasFile[];
}

/** `GET /api/import/capabilities` — what this screen may offer at all. */
export interface ImportCapabilities {
    readonly hledger: HledgerStatus;
    /** Lowercase extensions, no dots: `["csv","tsv","ofx",…]`. Drives the picker's `accept`. */
    readonly formats: readonly string[];
    readonly journals: readonly JournalTarget[];
    readonly git: GitCapability;
    /** Every `alias` the journal declares, in file order. Empty for most journals. */
    readonly aliases: readonly AliasEntry[];
    /** False ⇒ no journal is bound to an editor, so nothing here can be written. */
    readonly editable: boolean;
}

/**
 * Something the conversion DECIDED rather than read — a judgement call the user
 * is entitled to see. One variant per `convert::ConvertNote`, tagged by `kind`.
 */
export type ConvertNote =
    | {readonly kind: "sheetChosen"; readonly name: string; readonly of: number}
    | {readonly kind: "statementChosen"; readonly of: number}
    | {readonly kind: "datesFromSerial"; readonly count: number}
    | {readonly kind: "encodingGuessed"; readonly label: string}
    | {readonly kind: "delimiterSniffed"; readonly delimiter: string}
    | {readonly kind: "preambleSkipped"; readonly lines: number}
    | {readonly kind: "trailerSkipped"; readonly lines: number}
    | {readonly kind: "blankRowsDropped"; readonly count: number}
    | {readonly kind: "raggedRows"; readonly count: number}
    | {readonly kind: "balanceMismatch"; readonly expected: string; readonly computed: string};

/** The first rows of the converted CSV, as a table. */
export interface StagePreview {
    readonly header: readonly string[] | null;
    readonly rows: readonly (readonly string[])[];
    /** Rows in the WHOLE conversion, not in `rows` — `rows` is capped by `truncated`. */
    readonly rowCount: number;
    readonly truncated: boolean;
}

/** What the source format volunteered about the statement as a whole. Every field is optional. */
export interface StatementMeta {
    /** Masked to the last four characters by the engine; never a full account number. */
    readonly accountHint: string | null;
    readonly currency: string | null;
    /** Verbatim decimal TEXT — never parsed to a number on this side (convention #1). */
    readonly ledgerBalance: string | null;
    readonly balanceAsOf: string | null;
}

/** One transaction a rules file would produce, rendered by the engine for display only. */
export interface ProposedTxn {
    readonly date: string;
    readonly description: string;
    /** Pre-rendered posting lines. Text, not amounts — nothing here is ever summed. */
    readonly postings: readonly string[];
}

/**
 * The counts behind a candidate's score.
 *
 * The first five are the ones the contract's example carries and are required.
 * The last three exist on `matching::Signals` but are absent from the contract's
 * literal, so they decode to `null` = "this engine did not send it" rather than
 * to a made-up `0`/`false` that would read as a real measurement.
 */
export interface CandidateSignals {
    readonly txns: number;
    readonly postings: number;
    /** Fact 4: a posting hledger accepted with no amount at all. Silently broken. */
    readonly amountlessPostings: number;
    /** Fact 4: amounts with no commodity form a SEPARATE commodity, so the `$` balance never moves. */
    readonly bareCommodityAmounts: number;
    /** hledger's `expenses:unknown` / `income:unknown` fallback fired this many times. */
    readonly unknownAccounts: number;
    readonly emptyDescriptions: number | null;
    readonly columnCountMatches: boolean | null;
    readonly headerMatchesSource: boolean | null;
}

/** One ranked `*.rules` file, scored against the staged data. */
export interface RulesCandidate {
    /** The rules-file handle — the same id `/api/rules` uses. */
    readonly id: string;
    readonly label: string;
    /** 0.0..=1.0. Rendered as a percentage; never compared for equality. */
    readonly score: number;
    readonly signals: CandidateSignals;
    readonly sample: readonly ProposedTxn[];
    /**
     * The rules file's own top-level `account1` — the account every imported
     * posting lands in, and so the account a statement balance is a balance OF.
     * The balance field defaults to it. Null when the file declares none.
     */
    readonly account1: string | null;
    /** The file's top-level `account2`, on the same terms. */
    readonly account2: string | null;
}

/** The destinations the engine suggests. `journalId` is null when the journal offered none. */
export interface StageDefaults {
    readonly csvPath: string;
    readonly journalId: string | null;
}

/** `POST /api/import/stage` — the converted file, its preview, and the rules files that fit it. */
export interface StagedFile {
    /** Opaque token. NOT a path, and never resolvable to one by arithmetic. */
    readonly stageId: string;
    /** The detected `SourceFormat`, lowercase ("ofx", "xlsx", …). */
    readonly format: string;
    readonly preview: StagePreview;
    readonly statement: StatementMeta | null;
    readonly notes: readonly ConvertNote[];
    /**
     * Notes whose `kind` this build does not know, counted rather than dropped.
     *
     * Not a wire field — the decoder's own tally. A note is advisory and carries
     * no echo obligation (unlike a rules item, where an unknown `kind` THROWS
     * because the editor would otherwise claim an id it cannot render), so a
     * newer engine adding a variant must not cost the user the whole import.
     * Saying "1 note this build doesn't understand" is the honest middle.
     */
    readonly unknownNoteCount: number;
    /** Ranked best-first. EMPTY is a legitimate answer — see `noCandidates`. */
    readonly candidates: readonly RulesCandidate[];
    readonly defaults: StageDefaults;
}

/**
 * Rows `.latest` dropped, which hledger does SILENTLY.
 *
 * State lives beside the data file, keyed to its name, and holds the newest
 * imported date; a row older than that vanishes from the import with no mention
 * anywhere in hledger's output. Surfacing this is a definition-of-done item.
 */
export interface SkippedRows {
    readonly olderThan: string;
    readonly count: number;
}

/**
 * Statement balance vs. what the journal plus the proposed entries compute to.
 *
 * All four fields are decimal TEXT computed by the engine (by concatenation —
 * fact 3 — never two `-f` flags). Nothing on this side parses them: `matches` is
 * the engine's verdict and `difference` is its arithmetic.
 */
export interface BalanceCheck {
    readonly statement: string;
    readonly computed: string;
    readonly matches: boolean;
    /**
     * `statement - computed`, or null when the engine could not subtract them.
     *
     * A multi-commodity balance is the case that produces null: there is no one
     * number to report a gap as. The mismatch is still real and still shown —
     * only the size of it is unavailable, which is why `matches` is a separate
     * field rather than something derived from this one being "0.00".
     */
    readonly difference: string | null;
}

/** One clearing status a re-downloaded statement moved: an authorization hold that settled. */
export interface StatusChange {
    /** The row id, as the rules file wrote it (`comment id:%fitid`). */
    readonly id: string;
    /** What the journal says today: `unmarked`, `pending` or `cleared`. */
    readonly from: string;
    /** What this statement says. */
    readonly to: string;
    /**
     * Whether it was actually written. Always false on a dry run, which
     * previews and writes nothing. On a commit, false only for a match outside
     * the file this import writes to — syncing into some other included file
     * would write somewhere this request had neither checked nor could undo.
     */
    readonly applied: boolean;
}

/** One field a conflicting row disagrees on. */
export interface FieldDiff {
    /** What disagrees: `date`, `description`, `posting 2 amount`, … */
    readonly field: string;
    /** What the journal says today. */
    readonly existing: string;
    /** What this statement proposes. */
    readonly incoming: string;
}

/** One row the journal already holds differently — the hand-edit this feature exists to protect. */
export interface Conflict {
    /** The row id, as the rules file wrote it. */
    readonly id: string;
    /** Every disagreement, in field order. */
    readonly diffs: readonly FieldDiff[];
}

/**
 * What matching this statement's rows against the journal by id found, or null
 * when the rules file declares no id (`comment id:%fitid` — see `docs/imports.md`).
 *
 * Never an empty object when present: "there is no id to match on" and "there
 * is, and nothing matched" are different facts, and this type lets the UI tell
 * them apart. An id match may keep a row OUT of the import (already held) or
 * sync a clearing status; it never does anything else — a conflicting row is
 * reported and left exactly as the user wrote it, never overwritten, on the
 * premise that a field disagreement more likely means the journal was hand-
 * edited on purpose than that the bank's own data changed.
 */
export interface IdMatches {
    /** Rows no transaction in the journal claims. Imported as usual. */
    readonly new: number;
    /** Rows the journal already holds, identically. Not imported, not edited. */
    readonly unchanged: number;
    /** Status flips this statement would make (or, on commit, did make). */
    readonly statusChanged: readonly StatusChange[];
    /** How many there are — `statusChanged` may be capped by the engine. */
    readonly statusChangedTotal: number;
    /** Rows the journal holds differently in some way a status flip can't express. */
    readonly conflicting: readonly Conflict[];
    /** How many there are — `conflicting` may be capped by the engine. */
    readonly conflictingTotal: number;
}

/** A successful dry run: what hledger would write, and everything that must be seen before it is. */
export interface DryRunOk {
    readonly ok: true;
    /** hledger's stdout, VERBATIM — valid, re-parseable journal text. */
    readonly entries: string;
    readonly count: number;
    /** hledger's stderr status line, verbatim ("would import 3 new transactions from bank.csv:"). */
    readonly status: string;
    readonly skipped: SkippedRows | null;
    readonly balance: BalanceCheck | null;
    /**
     * What the journal's aliases did to THESE entries, or null when none is in
     * force. See {@link AliasEffect}.
     */
    readonly aliases: AliasEffect | null;
    /** Modified targets that make `commit` refuse. Empty when clear. */
    readonly blockedByGit: readonly string[];
    /**
     * The `ledgeline import …` line that reproduces this import from a terminal,
     * built by the engine's own argv builder — the same one `ledgeline import`
     * is parsed into, so what it says and what it does cannot drift.
     *
     * Carries relative handles only, so it is run from the journal's own
     * directory (which is what the panel says beside it). Not {@link CliParity},
     * which asks a different question about a different `cli`.
     */
    readonly cliCommand: string;
    /** What matching this statement against the journal by id found. See {@link IdMatches}. */
    readonly idMatches: IdMatches | null;
}

/** One account rewrite an alias performed on this import. */
export interface AliasRename {
    /** The account the rules file produced. */
    readonly from: string;
    /** The account the import will actually write. */
    readonly to: string;
}

/**
 * The account rewrites the forwarded aliases performed on this import.
 *
 * MEASURED by the engine, not inferred: it repeats the same dry run with no
 * `--alias` and diffs the two proposals, so these are hledger's own answer
 * rather than anyone's reimplementation of its regex engine.
 *
 * `renames` empty means the aliases matched nothing in this statement, which is
 * the screen's cue to stay quiet.
 */
export interface AliasEffect {
    readonly forwarded: number;
    readonly renames: readonly AliasRename[];
    /** Whether this same import, run from a terminal, would come out the same. */
    readonly cli: CliParity;
}

/** Why an alias cannot be written into a config file. */
export type ConfRefusalReason = "comment" | "replacementWhitespace" | "replacementBackslash" | "patternBracket" | "patternBackslash" | "patternSlash";

/** One alias that cannot be expressed in a config file, with the engine's reason. */
export interface ConfRefusal {
    readonly pattern: string;
    readonly replacement: string;
    readonly reason: ConfRefusalReason | null;
    readonly message: string;
}

/**
 * Would a plain command-line `hledger import` produce these accounts too?
 *
 * The question is real, not rhetorical. An `alias` directive in a journal is NOT
 * applied to an imported CSV — Ledgeline forwards it as `--alias`, which is the
 * only way it can reach one — so the same statement, the same rules file and the
 * same journal give a terminal `hledger import` different account names. An
 * `hledger.conf` closes that, because it applies to every hledger command.
 *
 * `matches` is the ENGINE's measurement, on the same principle as
 * {@link AliasEffect}'s renames: it repeats the import with exactly the aliases a
 * config file supplies and diffs the two proposals. Nothing compares alias
 * strings to decide it, so a user who hand-wrote an equivalent mapping in a
 * spelling of their own gets silence rather than a lecture.
 */
export interface CliParity {
    readonly matches: boolean;
    /** Command-line answer → Ledgeline's. Empty when `matches`. */
    readonly differences: readonly AliasRename[];
    /** The config in force, relative to the journal's directory (`hledger.conf`, `../hledger.conf`). Never absolute. */
    readonly confPath: string | null;
    /** It sits above the journal's directory, so the engine reads it and will not write it. */
    readonly confOutside: boolean;
    /** A command word the config forces on every hledger run, which breaks all of them. Null normally. */
    readonly confHijackedBy: string | null;
    /** The `--alias` lines the fix would add, shown before it is pressed — the conversion widens what matches. */
    readonly additions: readonly string[];
    readonly refusals: readonly ConfRefusal[];
    /** Echo this when installing the fix. Empty string is the revision of "no file yet". */
    readonly revision: string;
    readonly writable: boolean;
}

/** `POST /api/import/hledger-conf` — what the one-click fix wrote. */
export interface ConfWritten {
    readonly confPath: string;
    readonly created: boolean;
    readonly added: readonly string[];
    readonly revision: string;
}

/** A failed dry run. `stderr` is rendered verbatim in a `<pre>` and NEVER paraphrased. */
export interface DryRunFailed {
    readonly ok: false;
    readonly stderr: string;
}

export type DryRunResult = DryRunOk | DryRunFailed;

/** One transaction the re-sort would move, for the diff the user confirms. */
export interface SortMove {
    readonly date: string;
    readonly description: string;
    readonly fromLine: number;
    readonly toLine: number;
}

/** `hledger check ordereddates`, plus the plan that would fix it. */
export interface OrderingReport {
    readonly inOrder: boolean;
    readonly moves: readonly SortMove[];
}

/** What git did around the import. Null when no repository contained the targets. */
export interface GitReport {
    readonly committed: boolean;
    readonly paths: readonly string[];
    /** Targets deliberately not committed (gitignored, or in another repo). Reported, never force-added. */
    readonly skipped: readonly string[];
    /** A commit failure's stderr (a rejecting pre-commit hook, a GPG prompt, ...). Null on success. */
    readonly message: string | null;
}

/**
 * `POST /api/import/commit` — what was written.
 *
 * `journalWritten` is null for the Save-CSV-only path (no rules file chosen, so
 * no `hledger import` ran and no journal was touched); `imported` is 0 and
 * `ordering` is vacuously in order there.
 */
export interface CommitResult {
    readonly csvWritten: string;
    readonly journalWritten: string | null;
    readonly imported: number;
    readonly ordering: OrderingReport;
    readonly git: GitReport | null;
    /**
     * What matching this statement against the journal by id found, or null
     * when the rules file declares no id. See {@link IdMatches}. Unlike the
     * dry run's copy, `statusChanged[].applied` here reports what was actually
     * written, and `entries`/`imported` above are already net of it.
     */
    readonly idMatches: IdMatches | null;
}

/**
 * `POST /api/import/sort` — how many transactions the confirmed re-sort moved,
 * and what the git safety net did with the rewritten journal.
 *
 * The sort gets its own commit rather than amending the import's, so that
 * reverting the ordering and reverting the transactions are separate acts.
 * `git` is null when the journal is not under version control, when autocommit
 * is off, or when nothing moved.
 */
export interface SortResult {
    readonly moved: number;
    readonly git: GitReport | null;
}

/** `GET`/`PUT /api/prefs`. `gitAutocommit: null` = "commit when a repo is present". */
export interface Prefs {
    readonly hledgerPath: string | null;
    readonly gitAutocommit: boolean | null;
}

// ---------------------------------------------------------------------------
// QuickBooks Online Journal import (WP-17 Phase C)
//
// `StagedFile.format === "quickbooks-journal"` is the ONLY branch point this
// screen is allowed to make (see plans/17-quickbooks-journal-import.md's
// Phase C contract) — everything below is what the two dedicated routes,
// `GET`/`POST /api/import/qb-journal/*`, say once that branch is taken.
// `crates/ledgeline-server/src/qb_journal_api.rs`'s `Wire*` structs are the
// ground truth these mirror field by field.
// ---------------------------------------------------------------------------

/** hledger's own `%m/%d/%Y`-style guess, and whether the export gave enough evidence to be sure. */
export interface QbDateFormat {
    readonly format: string;
    /** True when nothing in the export rules out the other reading (day/month swapped). */
    readonly ambiguous: boolean;
}

/** One parsed QuickBooks transaction, flattened for display — text only, nothing here is summed. */
export interface QbSample {
    readonly id: string;
    readonly date: string;
    readonly description: string;
    readonly postings: readonly string[];
}

/**
 * What matching this export's transactions against the journal by id found —
 * the QuickBooks-import analogue of {@link IdMatches}. No `statusChanged`:
 * every transaction this pipeline builds is unmarked, so a status difference
 * from a hand-marked one is folded into `conflicting` rather than reported as
 * a sync (see `WireQbIdMatches`'s doc comment in `qb_journal_api.rs`).
 */
export interface QbIdMatches {
    readonly new: number;
    readonly unchanged: number;
    readonly conflicting: readonly Conflict[];
    readonly conflictingTotal: number;
}

/**
 * `GET /api/import/qb-journal/{stageId}` — a staged export's parsed groups,
 * its date-format guess, and which accounts are still unmapped.
 *
 * Read-only and idempotent: calling it again after adding an alias through
 * the existing `PUT /api/aliases/{*journalId}` is how `unmappedAccounts`
 * shrinks, and `idMatches` goes from null to populated once it is empty.
 */
export interface QbPreview {
    readonly stageId: string;
    readonly transactionCount: number;
    readonly postingCount: number;
    readonly dateFormat: QbDateFormat;
    /** Distinct QuickBooks account names no plain alias in the journal maps yet. Non-empty blocks a commit. */
    readonly unmappedAccounts: readonly string[];
    readonly sample: readonly QbSample[];
    /** Null while any account is unmapped — nothing can be built (and so nothing classified) without one. */
    readonly idMatches: QbIdMatches | null;
}

/** One `include`d file a commit touched, and whether it is still in date order after the write. */
export interface QbFileOrdering {
    /** A relative handle usable directly with the existing `POST /api/import/sort` route. */
    readonly journalId: string;
    readonly inOrder: boolean;
    readonly moves: readonly SortMove[];
}

/** Whether the journal is still in date order after the import, per touched file (a multi-year import can touch more than one). */
export interface QbOrdering {
    readonly inOrder: boolean;
    readonly files: readonly QbFileOrdering[];
}

/** `POST /api/import/qb-journal/commit` — what was written. */
export interface QbCommitResult {
    readonly imported: number;
    readonly idMatches: QbIdMatches;
    readonly ordering: QbOrdering;
    readonly git: GitReport | null;
}
