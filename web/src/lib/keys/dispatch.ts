// The heart of the keymap, and the reason the `?` sheet cannot lie.
//
// `resolveBindings` answers one question — "which bindings would fire right
// now, in the order the dispatcher tries them" — and BOTH consumers read it:
// `handleKey` searches the list, `helpSections` renders it. There is no second
// list to drift from, which is the single structural guarantee this feature
// rests on. `dispatch.test.ts` pins it with an explicit agreement test.
//
// Pure: no runes, no DOM. `keymap.svelte.ts` owns the state, this owns the
// decisions.

import {canonical, formatKeys, isPrefixOf, type KeyEventLike, type KeyToken, chordToken} from "./chord";
import {GROUP_ORDER, PRIORITY, type KeyGroup, type RegisteredLayer, type ResolvedBinding} from "./types";

/**
 * Every binding that can fire right now, best candidate first.
 *
 * Layers sort by priority descending, then registration order descending, so a
 * later-mounted widget at the same priority shadows an earlier one. The search
 * stops at the first `modal` layer: an overlay that owns the screen blinds
 * everything below it, for unmatched keys as much as matched ones.
 *
 * Then: drop anything `enabled()` says no to, and dedupe by canonical key,
 * first occurrence winning. The higher layer wins outright — there is no
 * negotiation and no `stopPropagation` anywhere in this design.
 */
export function resolveBindings(layers: readonly RegisteredLayer[]): ResolvedBinding[] {
    const ordered = [...layers].sort((a, b) => (b.priority ?? PRIORITY.page) - (a.priority ?? PRIORITY.page) || b.seq - a.seq);

    const resolved: ResolvedBinding[] = [];
    const claimed = new Set<string>();
    for (const layer of ordered) {
        for (const binding of layer.bindings) {
            if (binding.enabled !== undefined && !binding.enabled()) continue;
            const key = canonical(binding.keys);
            if (claimed.has(key)) continue;
            claimed.add(key);
            resolved.push({...binding, layerId: layer.id});
        }
        if (layer.modal === true) break;
    }
    return resolved;
}

export type Dispatch =
    | {kind: "run"; binding: ResolvedBinding}
    /** A prefix that some ENABLED binding continues. Swallow the key and wait. */
    | {kind: "pending"; sequence: string}
    /** A prefix was armed and this key does not continue it. Disarm, but let the key through. */
    | {kind: "clear"}
    /** Not ours. */
    | {kind: "ignore"};

/**
 * Decide what one keystroke means, given the resolved bindings and any armed
 * chord prefix.
 *
 * Exact match is tried across the WHOLE list before any prefix match, which is
 * what makes the shadowing rule total: a higher layer's `g` cannot steal a key
 * from a lower layer's completed `g j`, because the completed sequence is
 * checked first.
 */
export function handleKey(active: readonly ResolvedBinding[], pending: string, event: KeyEventLike): Dispatch {
    const token = chordToken(event);
    const sequence = pending === "" ? token : `${pending} ${token}`;

    const exact = active.find((binding) => canonical(binding.keys) === sequence);
    if (exact !== undefined) return {kind: "run", binding: exact};

    // A prefix only arms if something ENABLED would complete it — otherwise `g`
    // swallows the next key on behalf of a binding that cannot run.
    if (active.some((binding) => isPrefixOf(binding.keys, sequence))) return {kind: "pending", sequence};

    return pending === "" ? {kind: "ignore"} : {kind: "clear"};
}

export interface HelpRow {
    keys: string;
    label: string;
    tokens: KeyToken[];
}

export interface HelpSection {
    group: KeyGroup;
    rows: HelpRow[];
}

/**
 * The help sheet's content, from the same resolved list the dispatcher searches.
 *
 * Rows keep resolution order within a group (so the winning binding for a
 * shadowed key is the one shown, exactly once) and groups follow `GROUP_ORDER`.
 * An empty group is omitted rather than rendered as a bare heading.
 */
export function helpSections(active: readonly ResolvedBinding[]): HelpSection[] {
    const byGroup = new Map<KeyGroup, HelpRow[]>();
    for (const binding of active) {
        const rows = byGroup.get(binding.group) ?? [];
        rows.push({keys: binding.keys, label: binding.label, tokens: formatKeys(binding.keys)});
        byGroup.set(binding.group, rows);
    }
    return GROUP_ORDER.filter((group) => byGroup.has(group)).map((group) => ({group, rows: byGroup.get(group) ?? []}));
}
