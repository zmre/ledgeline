// A keystroke trace for e2e diagnosis: what the keymap saw and what it did
// with each key. Off unless a test puts an array at
// `window.__ledgelineKeyTrace` (Playwright's `addInitScript`), so the app pays
// one property read per keystroke and records nothing.
//
// Why it exists: review.e2e.ts drops a keystroke about once in 16 full runs
// (`s` once, ⇧A once), and the failure alone cannot say whether the key
// reached the keymap, which binding it matched, or which `enabled()` said no.

import {canonical, chordToken, type KeyEventLike} from "./chord";
import type {Dispatch} from "./dispatch";
import type {RegisteredLayer, ResolvedBinding} from "./types";

export interface KeyTraceEntry {
    /** `performance.now()`, ms. */
    at: number;
    key: string;
    target: string;
    /** `prevented`, `composing`, `field`, `run`, `pending`, `clear` or `ignore`. */
    outcome: string;
    /** The binding run, or why nothing ran. */
    detail?: string;
}

/** The live trace array, when a test asked for one. */
export function keyTrace(): KeyTraceEntry[] | undefined {
    if (typeof window === "undefined") return undefined;
    return (window as unknown as {__ledgelineKeyTrace?: KeyTraceEntry[]}).__ledgelineKeyTrace;
}

/** The target in words: tag, label or test id, and the review card it is on. */
function describeTarget(target: EventTarget | null): string {
    if (!(target instanceof Element)) return target === null ? "null" : "window";
    const name = target.getAttribute("aria-label") ?? target.getAttribute("data-testid") ?? "";
    const card = target.closest("[data-card-index]")?.getAttribute("data-card-index") ?? "-";
    return `${target.tagName.toLowerCase()} "${name}" card ${card}`;
}

/**
 * Every binding for this key in every layer, each with whether it is enabled
 * now, and whether a modal layer above blinds it — so an ignored key says
 * which condition turned its binding off. `stale` compares the cached resolved
 * list the keymap used against a fresh resolution: if they disagree, the
 * `$derived` was out of date when the key arrived.
 */
function whyIgnored(layers: readonly RegisteredLayer[], cached: readonly ResolvedBinding[], fresh: Dispatch, event: KeyEventLike): string {
    const key = chordToken(event);
    const candidates: string[] = [];
    for (const layer of layers) {
        for (const binding of layer.bindings) {
            if (canonical(binding.keys) !== key) continue;
            const enabled = binding.enabled === undefined ? "always-on" : binding.enabled() ? "enabled" : "disabled";
            candidates.push(`${layer.id}${layer.modal === true ? "(modal)" : ""}:${binding.label}=${enabled}`);
        }
    }
    const modals = layers.filter((layer) => layer.modal === true).map((layer) => `${layer.id}@${layer.priority ?? "page"}`);
    return [
        `token=${key}`,
        `candidates=[${candidates.join("; ")}]`,
        `modal-layers=[${modals.join(", ")}]`,
        `resolved=${cached.length}`,
        `fresh=${fresh.kind}${fresh.kind === "run" ? `(${fresh.binding.label})` : ""}`,
    ].join(" ");
}

export function recordKey(
    trace: KeyTraceEntry[],
    event: KeyboardEvent,
    outcome: string,
    context?: {layers: readonly RegisteredLayer[]; cached: readonly ResolvedBinding[]; fresh: Dispatch; decision?: Dispatch}
): void {
    let detail: string | undefined;
    if (context?.decision?.kind === "run") detail = context.decision.binding.label;
    else if (context !== undefined && (outcome === "ignore" || outcome === "clear")) detail = whyIgnored(context.layers, context.cached, context.fresh, event);
    trace.push({at: Math.round(performance.now()), key: event.key, target: describeTarget(event.target), outcome, detail});
}
