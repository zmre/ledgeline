// Display model for the grouped balance sheet (plans/12).
//
// Presentation, not engine: it lives under `reports/ui/` because collapsing a
// disclosure and demoting a commodity to a secondary line are decisions about a
// screen, and the purity rule (`reports/purity.test.ts`) keeps that out of the
// sources that port to Rust.
//
// The whole point of this module is that ONE function produces the list the
// template iterates AND the list the keyboard cursor indexes into, so the two
// cannot drift apart — the same discipline `ReportTable.svelte` documents for
// its own compressed rows. A row that is not in this list is not on screen and
// is not reachable with `j`.
//
// The current/non-current split inside Assets and Liabilities is ADAPTIVE, in
// the same sense as the income statement's GAAP ladder: a journal that carries
// no `bsterm:` tag gets no subheadings, no band subtotals, and precisely the
// rows this module emitted before the axis existed. `bsSectionBlocks` is where
// that promise is kept — and it is kept by an explicit early return, not by an
// accident of the walk.

import {maAdd, type MixedAmount} from "$lib/domain/money";
import type {BalanceSheetReport, BsGroup, BsSection, BsSectionKind, BsSubsection, BsTerm} from "$lib/reports/types";
import {compressSectionRows} from "./displayRows";

// `amountCell` lives in its own module now that the income statement renders
// its figures the same way; re-exported so the balance sheet's view and tests
// keep their one import (the pattern `insights/format.ts` already uses for the
// primitives it shares with the holdings UI).
export {amountCell, type AmountCell} from "./amountCell";

/**
 * A group heading (always shown), one of its accounts (only when expanded), a
 * current/non-current SUBHEADING, or that band's SUBTOTAL.
 *
 * The last two are ADAPTIVE: they exist only for a section the engine sent
 * `subsections` for. A journal that tags nothing produces exactly the first two
 * kinds, in exactly the order it produced them before this axis existed.
 */
export type BsRowKind = "group" | "account" | "subsection" | "subtotal";

export interface BsDisplayRow {
    kind: BsRowKind;
    /**
     * Stable identity, unique across the whole report. Used for three things at
     * once: the `{#each}` key, the collapse set's membership, and the keyboard
     * cursor's anchor. One key rather than three keeps them in agreement when
     * a refetch replaces the report with an equal-but-not-identical one.
     */
    key: string;
    /**
     * The group's name, the compressed account label relative to its displayed
     * parent, or the engine's own `heading` / `label` for the two band rows —
     * never a string composed here.
     */
    label: string;
    /** 0 for a group heading and for both band rows; 1 + the compression indent for accounts. */
    indent: number;
    /** The account this row stands for — `data-account` and the journal drill-down. Null for everything else. */
    account: string | null;
    /**
     * The figure on the right: a group subtotal, an account's subaccount-inclusive
     * balance, or a band's engine-computed subtotal. EMPTY on a subheading row,
     * which carries no figure of its own — the band's total belongs to the
     * subtotal row that closes it, and printing it twice would invite the reader
     * to look for a difference between them.
     */
    amount: MixedAmount;
    /**
     * The band this row belongs to, or null when the journal classifies nothing.
     *
     * Non-null on every `subsection`/`subtotal` row (each one IS a band) and on
     * the groups inside one; always null on an account row, which is read as
     * part of the group above it rather than as a line of the statement.
     */
    term: BsTerm | null;
    /** Group rows: whether the disclosure is currently open. Always false for an account row. */
    expanded: boolean;
    /**
     * Whether the disclosure can open at all. A COMPUTED group (Retained
     * earnings, Valuation adjustment) summarizes accounts that are not on the
     * balance sheet, so it has a total and no rows — it must render as a plain
     * line, not as a triangle that does nothing when clicked.
     */
    expandable: boolean;
}

/**
 * The spreadsheet-style tie-out under the three boxes: the three section
 * totals, then `Liabilities + equity` set against `Total assets`, then net
 * worth as its own figure.
 *
 * Why a tie-out rather than the bare net-worth panel this replaces: by
 * construction `Total equity ≡ Assets − Liabilities ≡ Net worth`, so a panel
 * showing only net worth restated a number already on screen and proved
 * nothing. Showing `L + E` against `A` is the check a reader of a balance sheet
 * actually performs, and it is the line that carries the verdict.
 *
 * `liabilitiesPlusEquity` is summed from the exact `Dec` values and is for
 * DISPLAY ONLY. The verdict is `report.balanced`, passed through untouched:
 * re-deriving it from rendered strings would round a real half-cent imbalance
 * into "balanced", and re-deriving it from `report.check` would do the opposite
 * — flag every journal holding a fractional lot, whose at-cost sum carries
 * unavoidable sub-cent dust (see the engine's `is_balanced`).
 */
export interface BsSummary {
    assets: MixedAmount;
    liabilities: MixedAmount;
    equity: MixedAmount;
    /** `liabilities + equity` — what must equal `assets`. Display only. */
    liabilitiesPlusEquity: MixedAmount;
    /** `assets − liabilities`, straight from the engine. */
    netWorth: MixedAmount;
    /** `report.balanced` — the engine's verdict, never a re-derived one. */
    balanced: boolean;
}

/** Build the tie-out. Shared by the view and the xlsx export so they cannot drift. */
export function bsSummary(report: BalanceSheetReport): BsSummary {
    // By kind, not by index: the engine always emits three sections in order,
    // and looking them up by name means a change to that order would move the
    // labels and the figures together rather than silently mislabelling one.
    const total = (kind: BsSectionKind): MixedAmount => report.sections.find((section) => section.kind === kind)?.total ?? new Map();
    const liabilities = total("liabilities");
    const equity = total("equity");
    return {
        assets: total("assets"),
        liabilities,
        equity,
        liabilitiesPlusEquity: maAdd(liabilities, equity),
        netWorth: report.netWorth,
        balanced: report.balanced,
    };
}

/**
 * The collapse-set key for one group. Exported so a caller can seed or assert
 * the set.
 *
 * The TERM is part of it, because the engine keys groups by `(term, name)`: one
 * `bsgroup:` whose accounts are partly current and partly not prints as two
 * lines under two subheadings — a receivable due this year and one due in five
 * are two lines on a real statement. Section + name alone would give those two
 * rows ONE key, and this key is doing three jobs at once: they would share a
 * collapse state, share a cursor stop, and be a duplicate `{#each}` key.
 *
 * An unclassified group keeps the two-segment key it has always had, so an
 * untagged journal's keys are unchanged along with everything else about it.
 */
export function bsGroupKey(sectionKind: string, groupName: string, term: BsTerm | null = null): string {
    return term === null ? `${sectionKind}/${groupName}` : `${sectionKind}/${term}/${groupName}`;
}

/**
 * The row key for a band's subheading. `occurrence` is which OPENING of the
 * band this is — 1 everywhere the engine's contiguity contract holds, and
 * occurrence 1 is unsuffixed on purpose, so a well-formed report's keys are
 * byte-for-byte what they have always been.
 *
 * The band rows share the group keyspace (`{kind}/{name}`), so they take an `@`
 * sigil: `bsgroup:` values are free text, and a group named exactly "@current"
 * is the only way to collide with one. An earlier version of this comment
 * called such a collision "a duplicate `{#each}` key and nothing else" — wrong:
 * Svelte 5's keyed `{#each}` throws `each_key_duplicate` in dev and misrenders
 * in prod, so one duplicated key blanks the whole statement. That severity is
 * why a band that opens a second time (the non-contiguity break — see
 * `bsSectionBlocks`) numbers its later openings into the key rather than
 * reusing the first one.
 */
export function bsSubsectionKey(sectionKind: string, term: BsTerm, occurrence = 1): string {
    const key = `${sectionKind}/@${term}`;
    return occurrence === 1 ? key : `${key}#${occurrence}`;
}

/** The row key for a band's subtotal — occurrence-numbered with its subheading. */
export function bsSubtotalKey(sectionKind: string, term: BsTerm, occurrence = 1): string {
    return `${bsSubsectionKey(sectionKind, term, occurrence)}/total`;
}

/**
 * One group, plus the band that opens before its line and the band that closes
 * after it — the whole current/non-current layout decision, in one place.
 *
 * Shared with the xlsx export rather than walked twice. The strings are the
 * engine's either way, so what would actually have been duplicated is *where the
 * boundaries fall*, and a workbook that split its bands one group earlier than
 * the screen is a subtler wrong than a mislabelled one.
 */
export interface BsSectionBlock {
    /** The band whose subheading prints above this group, or null when the group continues one. */
    opens: BsSubsection | null;
    group: BsGroup;
    /** The band whose subtotal prints below this group, or null when the band runs on. */
    closes: BsSubsection | null;
}

/**
 * Lay a section's groups out into bands.
 *
 * `subsections` empty — the untagged journal, and every equity section — returns
 * each group with no band on either side, which is byte-for-byte the layout this
 * module produced before the axis existed. That is the whole adaptive guarantee,
 * and it is one branch rather than an emergent property of the walk below.
 *
 * Otherwise the walk leans on the engine's ordering invariant (groups of one
 * term are contiguous): a band opens at the first group whose term differs from
 * its predecessor's and closes at the last group whose term differs from its
 * successor's. Three things the contract says cannot happen are nonetheless
 * handled in the direction that loses nothing. A group whose term names no
 * band, and a group with no term at all, both still render their own line, just
 * without a heading over them. And groups of one term arriving NON-CONTIGUOUSLY
 * — [current, noncurrent, current] — simply RE-OPEN the band: the stray run
 * gets the heading and the engine's subtotal again, in the engine's own order.
 * Coalescing the runs into one band would have made each subtotal truthful
 * about exactly the rows above it, but only by silently REORDERING the engine's
 * groups — and this module reorders nothing anywhere else, so a band that
 * visibly opens twice is honest about the broken input where a quietly
 * relocated group would repair it behind the reader's back. The one thing a
 * re-opened band must not do is reuse its first opening's row keys, which
 * Svelte punishes as a blank statement rather than tolerating (see
 * `bsSubsectionKey`); `sectionDisplayRows` numbers the openings for that.
 * A band with no groups is never emitted, so a subheading can never stand over
 * nothing.
 */
export function bsSectionBlocks(section: BsSection): BsSectionBlock[] {
    const groups = section.groups;
    if (section.subsections.length === 0) return groups.map((group) => ({opens: null, group, closes: null}));

    const byTerm = new Map(section.subsections.map((subsection) => [subsection.term, subsection]));
    const edge = (term: BsTerm | null, neighbour: BsTerm | null): BsSubsection | null =>
        term === null || term === neighbour ? null : (byTerm.get(term) ?? null);

    return groups.map((group, i) => ({
        opens: edge(group.term, i === 0 ? null : groups[i - 1].term),
        group,
        closes: edge(group.term, i === groups.length - 1 ? null : groups[i + 1].term),
    }));
}

/**
 * One section's visible rows, in visual order: a band subheading where one
 * opens, every group heading, each followed by its accounts when it is expanded,
 * and the band's subtotal where one closes.
 *
 * Account rows go through `compressSectionRows`, so a single-child chain
 * (`assets` → `assets:bank`) reads as one row here exactly as it does in every
 * other report table.
 *
 * The subtotal closes a band AFTER its last group's accounts, not before them:
 * an expanded disclosure is part of the group it belongs to, and a subtotal
 * printed above the accounts it contains would read as excluding them.
 */
export function sectionDisplayRows(section: BsSection, isExpanded: (key: string) => boolean): BsDisplayRow[] {
    const out: BsDisplayRow[] = [];
    // Which opening of each band we are inside. Stays at 1 for any section the
    // engine's contiguity contract holds for; it passes 1 only on the
    // non-contiguity break (see `bsSectionBlocks`), where the occurrence number
    // is what keeps a re-opened band's two rows from reusing the first
    // opening's keys.
    const openings = new Map<BsTerm, number>();
    for (const {opens, group, closes} of bsSectionBlocks(section)) {
        if (opens !== null) {
            const occurrence = (openings.get(opens.term) ?? 0) + 1;
            openings.set(opens.term, occurrence);
            out.push({
                kind: "subsection",
                key: bsSubsectionKey(section.kind, opens.term, occurrence),
                label: opens.heading,
                indent: 0,
                account: null,
                amount: new Map(),
                term: opens.term,
                expanded: false,
                expandable: false,
            });
        }

        const key = bsGroupKey(section.kind, group.name, group.term);
        const expandable = group.rows.length > 0;
        const expanded = expandable && isExpanded(key);
        out.push({kind: "group", key, label: group.name, indent: 0, account: null, amount: group.total, term: group.term, expanded, expandable});
        if (expanded) {
            for (const display of compressSectionRows(group.rows)) {
                out.push({
                    kind: "account",
                    key: `${key}/${display.row.account}`,
                    label: display.label,
                    indent: display.indent + 1,
                    account: display.row.account,
                    // `inclusive`, not `own`: a displayed row stands for its whole
                    // subtree, which is what the depth clamp left visible.
                    amount: display.row.inclusive,
                    term: null,
                    expanded: false,
                    expandable: false,
                });
            }
        }

        if (closes !== null) {
            out.push({
                kind: "subtotal",
                // A subtotal closes the opening it sits inside, so the count at
                // close time IS this row's occurrence. (`?? 1` satisfies the
                // types, not a reachable case: a run's first group always fires
                // `opens` before its last fires `closes`.)
                key: bsSubtotalKey(section.kind, closes.term, openings.get(closes.term) ?? 1),
                label: closes.label,
                // The engine's own subtotal, passed through by reference. Summing
                // the group lines here would re-add figures already rounded for
                // display, which is how a band comes to a cent off the section
                // total printed under it.
                amount: closes.total,
                indent: 0,
                account: null,
                term: closes.term,
                expanded: false,
                expandable: false,
            });
        }
    }
    return out;
}

/**
 * Every cursorable row, in visual order — a filtering of the very arrays the
 * template iterates, so a row can never be reachable by `j` and absent from the
 * screen.
 *
 * Band rows are deliberately NOT in it, for the reason the income statement
 * keeps its ladder lines out of `isCursorRows`: neither a subheading nor a
 * subtotal can be expanded or drilled into, so landing on one is a stop where
 * Enter does nothing.
 */
export function bsCursorRows(rows: readonly BsDisplayRow[]): BsDisplayRow[] {
    return rows.filter((row) => row.kind === "group" || row.kind === "account");
}
