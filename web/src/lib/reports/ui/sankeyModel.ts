// The two money-flow diagrams, as plain data.
//
// `reports/ui/` is exempt from the WP-06 purity rule, so this may reach into
// `$lib/stores/loadState` for the panel's tri-state. It still imports no
// Svelte and touches no DOM: everything here is a pure function of a decoded
// `FlowReport`, which is what lets the folding and colour rules be tested
// without mounting anything.
//
// Two decisions the shapes below encode:
//
//   1. COLOUR IDENTITY IS THE ACCOUNT, ACROSS BOTH GRAPHS. The palette is
//      computed over the whole report rather than per graph, so
//      `assets:bank:checking` is the same blue in "Money in" as in "Money out".
//      Per-graph slots would hand the same account two hues on one screen, and
//      the two diagrams sit one above the other.
//   2. STATEMENT-SIDE NODES NEVER FOLD AND NEVER TAKE A SLOT. Folding them would
//      hide exactly the spending categories the diagram exists to show. Only the
//      account side has a tail, and it is the side whose identity a reader can
//      recover from the legend.

import {add, cmp, toNumber, type Dec} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import {fmt} from "$lib/format/amounts";
import {CATEGORICAL, colorAt, OTHER_COLOR, OTHER_LABEL} from "$lib/format/palette";
import type {FlowGraph, FlowReport, FlowSide} from "$lib/reports/types";
import type {DataView} from "$lib/stores/loadState";

/**
 * The key the folded tail is drawn under. Namespaced apart from the engine's
 * own `g:`/`a:` keys so an account literally named "(other)" cannot collide
 * with the bucket.
 */
export const OTHER_KEY = "x:other";

/** The colour an account-side node keeps in BOTH graphs, and which ones fold away. */
export interface FlowPalette {
    /** The slot colour for an account key; `OTHER_COLOR` for a folded or unknown one. */
    color(key: string): string;
    /** Whether this account key is past the last slot, and so folds into the tail bucket. */
    folded(key: string): boolean;
    /**
     * Slot index, or `CATEGORICAL.length` for a folded key. This is what puts a
     * graph's legend in PALETTE order rather than in its own node order: an
     * account can rank third by combined total and first within one graph.
     */
    rank(key: string): number;
}

/**
 * Assign a palette slot to every account in the report, biggest combined total
 * first.
 *
 * The tail past the last slot is `folded`, never cycled back onto slot 1 (see
 * `$lib/format/palette`): `sankeyView` is the caller that has to fold it into
 * an `OTHER_LABEL` bucket, and `colorAt` is only the backstop.
 */
export function flowPalette(report: FlowReport): FlowPalette {
    const totals = new Map<string, Dec>();
    for (const graph of [report.inflows, report.outflows]) {
        for (const node of graph.nodes) {
            if (node.account === null) continue;
            const seen = totals.get(node.key);
            totals.set(node.key, seen === undefined ? node.total : add(seen, node.total));
        }
    }
    // Ties broken by key so a redraw cannot reshuffle two equal accounts.
    const ranked = [...totals].sort(([aKey, aTotal], [bKey, bTotal]) => cmp(bTotal, aTotal) || (aKey < bKey ? -1 : aKey > bKey ? 1 : 0));
    const slots = new Map(ranked.map(([key], i) => [key, i]));
    const rank = (key: string): number => Math.min(slots.get(key) ?? CATEGORICAL.length, CATEGORICAL.length);
    return {
        color: (key) => colorAt(rank(key)),
        folded: (key) => (slots.get(key) ?? -1) >= CATEGORICAL.length,
        rank,
    };
}

/** One bar. `color` is null for a statement line, which the panel paints neutral. */
export interface SankeyNodeView {
    key: string;
    label: string;
    side: FlowSide;
    /**
     * The account this bar stands for. Null for a statement line AND for the
     * folded tail, which stands for several accounts and so names none.
     */
    account: string | null;
    value: number;
    amount: string;
    /** Percent of the graph's drawn total; 0 when that total is zero. */
    share: number;
    color: string | null;
}

/** One ribbon. `color` is the account end's colour, whichever side that is in this graph. */
export interface SankeyLinkView {
    source: string;
    target: string;
    value: number;
    color: string;
    title: string;
}

export interface SankeyView {
    nodes: SankeyNodeView[];
    links: SankeyLinkView[];
    total: string;
    sectionTotal: string;
    /** Whether the drawn links account for the whole statement figure. */
    complete: boolean;
    legend: {key: string; label: string; color: string; amount: string}[];
}

/**
 * One graph, ready to draw: the tail folded, the links re-aggregated behind it,
 * and every figure already a `number` or a `string`.
 *
 * Nothing here may hold a `Dec` or a `bigint`. LayerChart's `Sankey` runs
 * `structuredClone` over the graph it is handed, and the layout needs plain
 * numbers anyway, so `toNumber` is used at exactly this boundary (which is what
 * the money contract documents it for).
 */
export function sankeyView(graph: FlowGraph, palette: FlowPalette, base: string | null, styles: ReadonlyMap<string, AmountStyle>): SankeyView {
    // `fmt`, not the statement's `amountCell`: every figure in this report is
    // ONE `Dec` in `base`, where `amountCell` takes a whole `MixedAmount` and
    // exists to demote the commodities a valuation could not reach. There are
    // none to demote here. An empty commodity renders the bare number, which is
    // the honest reading when the journal has no base.
    const money = (value: Dec): string => fmt(base ?? "", value, styles);
    const drawn = toNumber(graph.total);
    const share = (value: Dec): number => (drawn === 0 ? 0 : (toNumber(value) / drawn) * 100);

    const nodes: SankeyNodeView[] = [];
    const labels = new Map<string, string>();
    // Every account node in one graph sits on the same side (targets in "Money
    // in", sources in "Money out"), so the tail inherits it from the first one.
    let tail: {side: FlowSide; total: Dec} | null = null;

    for (const node of graph.nodes) {
        if (node.account !== null && palette.folded(node.key)) {
            tail = {side: node.side, total: tail === null ? node.total : add(tail.total, node.total)};
            continue;
        }
        labels.set(node.key, node.label);
        nodes.push({
            key: node.key,
            label: node.label,
            side: node.side,
            account: node.account,
            value: toNumber(node.total),
            amount: money(node.total),
            share: share(node.total),
            color: node.account === null ? null : palette.color(node.key),
        });
    }
    if (tail !== null) {
        labels.set(OTHER_KEY, OTHER_LABEL);
        nodes.push({
            key: OTHER_KEY,
            label: OTHER_LABEL,
            side: tail.side,
            account: null,
            value: toNumber(tail.total),
            amount: money(tail.total),
            share: share(tail.total),
            color: OTHER_COLOR,
        });
    }

    // Several folded accounts feeding one statement line become ONE ribbon.
    const fold = (key: string): string => (palette.folded(key) ? OTHER_KEY : key);
    const merged = new Map<string, {source: string; target: string; value: Dec}>();
    for (const link of graph.links) {
        const source = fold(link.source);
        const target = fold(link.target);
        const id = `${source}\u0000${target}`;
        const seen = merged.get(id);
        if (seen === undefined) merged.set(id, {source, target, value: link.value});
        else seen.value = add(seen.value, link.value);
    }

    const accountSide: FlowSide = graph.nodes.find((node) => node.account !== null)?.side ?? "source";
    const links: SankeyLinkView[] = [...merged.values()].map(({source, target, value}) => ({
        source,
        target,
        value: toNumber(value),
        color: palette.color(accountSide === "source" ? source : target),
        title: `${labels.get(source) ?? source} → ${labels.get(target) ?? target}: ${money(value)}`,
    }));

    const legend = nodes
        .filter((node): node is SankeyNodeView & {color: string} => node.color !== null)
        .sort((a, b) => palette.rank(a.key) - palette.rank(b.key))
        .map((node) => ({key: node.key, label: node.label, color: node.color, amount: node.amount}));

    return {
        nodes,
        links,
        // The exact `Dec`s, never the formatted strings: two figures a cent
        // apart round to the same "$34,010.00" at display precision, and this
        // flag is the only thing that says the picture is partial.
        complete: cmp(graph.total, graph.sectionTotal) === 0,
        total: money(graph.total),
        sectionTotal: money(graph.sectionTotal),
        legend,
    };
}

/** Everything the two panels need, threaded from the page as one value. */
export interface FlowsPanel {
    view: DataView;
    report: FlowReport | null;
    error: Error | null;
    retry: () => void;
}
