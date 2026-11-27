// The Value chart's bought/sold markers (plan 47 A1e). Pure — no Svelte
// runtime — so the threshold is node-tested.
//
// A value line jumps when money moves in or out, and without a mark a $48k
// sale reads exactly like a $48k loss. So a bucket whose NET flow is large
// against the chart's own scale gets a ▲ (net bought) or ▼ (net sold) on the
// line. "Large" is relative to what the reader can see: 5 % of the y-axis
// span. Dividend reinvestment and a monthly contribution into a big portfolio
// stay unmarked — on that axis they are noise, and marking every one would
// bury the sale that explains a dip.

/** Share of the y-axis span a bucket's net flow must reach to be marked. */
export const FLOW_MARK_SHARE = 0.05;

/** One marked bucket. */
export interface FlowMarker {
    /** Bucket index on the chart. */
    index: number;
    /** "in" = net bought (▲), "out" = net sold (▼). */
    direction: "in" | "out";
    /** The bucket's net flow (positive in, negative out), in the base commodity. */
    flow: number;
    /** The line's value at the bucket: where the marker sits. */
    value: number;
}

/**
 * The buckets to mark. `flows` and `values` are index-aligned to the chart's
 * points; `domain` is the y-axis the line is drawn against (null when the
 * chart computes its own, in which case the values' extent with zero, which is
 * what it draws against, stands in). A bucket with no value has no point on
 * the line to carry a marker and is skipped, as is a zero flow.
 */
export function flowMarkers(flows: readonly number[], values: readonly (number | null)[], domain: readonly [number, number] | null): FlowMarker[] {
    const span = domainSpan(values, domain);
    if (span <= 0) return [];
    const threshold = span * FLOW_MARK_SHARE;
    return flows.flatMap((flow, index) => {
        const value = values[index] ?? null;
        if (value === null || !Number.isFinite(flow) || flow === 0 || Math.abs(flow) < threshold) return [];
        return [{index, direction: flow > 0 ? "in" : "out", flow, value}];
    });
}

function domainSpan(values: readonly (number | null)[], domain: readonly [number, number] | null): number {
    if (domain !== null) return domain[1] - domain[0];
    const present = values.filter((n): n is number => n !== null && Number.isFinite(n));
    if (present.length === 0) return 0;
    return Math.max(0, ...present) - Math.min(0, ...present);
}

/** The tooltip line for a marked bucket: "Net bought $12,000" / "Net sold $48,200" (the caller picks the precision). */
export function flowLine(marker: Pick<FlowMarker, "flow">, formatMoney: (n: number) => string): string {
    return `${marker.flow > 0 ? "Net bought" : "Net sold"} ${formatMoney(Math.abs(marker.flow))}`;
}
