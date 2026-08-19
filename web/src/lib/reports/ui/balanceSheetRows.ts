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

import {maAdd, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {fmt} from "$lib/format/amounts";
import type {BalanceSheetReport, BsSection, BsSectionKind} from "$lib/reports/types";
import {compressSectionRows} from "./displayRows";
import {extras as nonBaseLines, fmtBase} from "./insights/format";

/** A group heading (always shown) or one of its accounts (only when expanded). */
export type BsRowKind = "group" | "account";

export interface BsDisplayRow {
    kind: BsRowKind;
    /**
     * Stable identity, unique across the whole report. Used for three things at
     * once: the `{#each}` key, the collapse set's membership, and the keyboard
     * cursor's anchor. One key rather than three keeps them in agreement when
     * a refetch replaces the report with an equal-but-not-identical one.
     */
    key: string;
    /** The group's name, or the compressed account label relative to its displayed parent. */
    label: string;
    /** 0 for a group heading; 1 + the compression indent for its accounts. */
    indent: number;
    /** The account this row stands for — `data-account` and the journal drill-down. Null for a group. */
    account: string | null;
    /** The figure on the right: a group subtotal, or an account's subaccount-inclusive balance. */
    amount: MixedAmount;
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

/** The collapse-set key for one group. Exported so a caller can seed or assert the set. */
export function bsGroupKey(sectionKind: string, groupName: string): string {
    return `${sectionKind}/${groupName}`;
}

/**
 * One section's visible rows, in visual order: every group heading, each
 * followed by its depth-clamped accounts when it is expanded.
 *
 * Account rows go through `compressSectionRows`, so a single-child chain
 * (`assets` → `assets:bank`) reads as one row here exactly as it does in every
 * other report table.
 */
export function sectionDisplayRows(section: BsSection, isExpanded: (key: string) => boolean): BsDisplayRow[] {
    const out: BsDisplayRow[] = [];
    for (const group of section.groups) {
        const key = bsGroupKey(section.kind, group.name);
        const expandable = group.rows.length > 0;
        const expanded = expandable && isExpanded(key);
        out.push({kind: "group", key, label: group.name, indent: 0, account: null, amount: group.total, expanded, expandable});
        if (!expanded) continue;
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
                expanded: false,
                expandable: false,
            });
        }
    }
    return out;
}

/**
 * One amount rendered the way the insights dashboard renders one: a single
 * dominant figure in the base commodity, everything else demoted to a small
 * secondary line.
 *
 * This replaces `formatTotals`' stack of `<div>`s — with every line valued into
 * one commodity, a balance sheet cell has exactly one number in it, and the
 * leftovers (an unpriced holding the valuation could not convert) are an
 * annotation rather than a second balance.
 */
export interface AmountCell {
    /** The headline figure. A real formatted zero ("$0.00") when the amount has no base part. */
    text: string;
    /** Whether `text` is negative — the caller paints it `text-error`. */
    negative: boolean;
    /** Non-base commodities, formatted, sorted, zeroes dropped. */
    extras: string[];
}

/**
 * `base` may be null: the engine reports no base commodity for a journal that
 * has none, and there is then no figure to promote. Rather than invent one, the
 * first commodity (in sort order) becomes the headline and the rest stay extras
 * — deterministic, and honest that nothing was converted.
 */
export function amountCell(ma: MixedAmount, base: string | null, styles: ReadonlyMap<string, AmountStyle>): AmountCell {
    if (base !== null) {
        const qty = ma.get(base);
        return {text: fmtBase(ma, base, styles), negative: qty !== undefined && qty.m < 0n, extras: nonBaseLines(ma, base, styles)};
    }
    const sorted = [...ma.entries()].filter(([, qty]) => qty.m !== 0n).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    if (sorted.length === 0) return {text: "0", negative: false, extras: []};
    const [commodity, qty] = sorted[0];
    return {
        text: fmt(commodity, qty, styles),
        negative: qty.m < 0n,
        extras: sorted.slice(1).map(([c, q]) => fmt(c, q, styles)),
    };
}
