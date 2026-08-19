// The keymap store: the layer stack, the armed chord prefix, the help sheet's
// open state, and the one window-level handler the whole app routes through.
//
// This is the app's FIRST global key handling of any kind. The listener lives in
// `+layout.svelte` as `<svelte:window onkeydown>`, which is the BUBBLE phase —
// so all four pre-existing element-scoped handlers (AccountInput, TransactionRow,
// TransactionModal, ColumnMenu) run first and unchanged. Because they already
// `preventDefault()` the keys they claim, `event.defaultPrevented` is a free
// cooperation protocol: no `stopPropagation` is introduced anywhere, preserving
// that property of this codebase.
//
// ColumnMenu's "attached only while open so there is no idle document listener"
// rule is about CONDITIONAL listeners. This one is unconditional for the app's
// lifetime, so `<svelte:window>` is the right home — an `$effect` +
// `addEventListener` would reimplement it with a manual teardown to get wrong.

import {untrack} from "svelte";
import {handleKey, helpSections, resolveBindings, type HelpSection} from "./dispatch";
import {isTypingTarget, type TargetLike} from "./target";
import {CHORD_TIMEOUT_MS, type Layer, type RegisteredLayer, type ResolvedBinding} from "./types";

let layers = $state<RegisteredLayer[]>([]);
let pending = $state("");
let helpOpen = $state(false);

let seq = 0;
let pendingTimer: ReturnType<typeof setTimeout> | undefined;

const active = $derived.by(() => resolveBindings(layers));
const help = $derived.by(() => helpSections(active));

function disarm(): void {
    if (pendingTimer !== undefined) {
        clearTimeout(pendingTimer);
        pendingTimer = undefined;
    }
    if (pending !== "") pending = "";
}

function arm(sequence: string): void {
    pending = sequence;
    if (pendingTimer !== undefined) clearTimeout(pendingTimer);
    // `setTimeout`, never `Date.now()` arithmetic: the e2e suite runs under
    // `page.clock.setFixedTime`, which freezes `Date` while leaving timers
    // running. Same reasoning as SearchInput's debounce and the table's pulse.
    pendingTimer = setTimeout(disarm, CHORD_TIMEOUT_MS);
}

/**
 * Register a keymap layer for the lifetime of the calling component.
 *
 * Must be called during component initialization — it declares an `$effect`,
 * the same contract as `onServerReady`. The effect body reads nothing reactive,
 * so it registers exactly once and its cleanup is the unregistration. Pass a
 * plain object: anything dynamic belongs in a binding's `enabled` predicate, not
 * in a `$derived` layer that would re-register on every tick.
 */
export function registerKeys(layer: Layer): void {
    $effect(() => {
        seq += 1;
        // Captured in a local: `seq` keeps incrementing, so the teardown below
        // must close over THIS registration's number, not read the counter later.
        const id = seq;
        const entry: RegisteredLayer = {...layer, seq: id};
        // `untrack` is load-bearing, not decoration. Without it this effect READS
        // `layers` (to copy it) and WRITES `layers`, which is precisely the
        // self-feeding shape that produced `effect_update_depth_exceeded` and
        // froze the app in AliasPanel — see the comment at AliasPanel.svelte:54.
        // Here it froze every keymap test instead, which is the better place to
        // find out. The effect must run exactly once per mount; nothing it reads
        // should re-trigger it.
        untrack(() => {
            layers = [...layers, entry];
        });
        return () => {
            untrack(() => {
                // Filter on `seq`, NOT on object identity (`!== entry`). `$state`
                // hands back a PROXY of each element, so the proxy is never
                // reference-equal to the raw object we pushed and an
                // identity filter silently removes nothing — layers accumulate
                // forever and an unmounted component's bindings keep firing.
                layers = layers.filter((candidate) => candidate.seq !== id);
            });
        };
    });
}

export const keymap = {
    /** Every binding that would fire right now, best candidate first. */
    get active(): ResolvedBinding[] {
        return active;
    },
    /** The same list, grouped for the `?` sheet. Never a second source of truth. */
    get help(): HelpSection[] {
        return help;
    },
    /** The armed chord prefix (`"g"`), or `""`. Rendered by ChordIndicator. */
    get pending(): string {
        return pending;
    },
    get helpOpen(): boolean {
        return helpOpen;
    },
    openHelp(): void {
        helpOpen = true;
    },
    closeHelp(): void {
        helpOpen = false;
    },
    toggleHelp(): void {
        helpOpen = !helpOpen;
    },
    /** Drop any armed chord prefix. Also bound to window blur, for Cmd-Tab. */
    disarm,

    /** The one keydown entry point. Wired in `+layout.svelte`. */
    handle(event: KeyboardEvent): void {
        // A local handler already claimed it (see the file header).
        if (event.defaultPrevented) return;
        // Mid-IME composition, and the dead-key half of an accented character
        // (Option-e on a US layout in the WKWebView) — neither is a keystroke yet.
        if (event.isComposing || event.key === "Dead" || event.key === "Unidentified") return;
        // Focus is in a field: the app is not listening, with NO exceptions,
        // Escape included. Escape inside a field belongs to that field, which is
        // the shape the four pre-existing handlers already have. One rule with
        // no carve-outs cannot drift.
        if (isTypingTarget(event.target as TargetLike | null)) {
            disarm();
            return;
        }
        // Escape always disarms a half-typed chord, then falls through so an
        // Escape BINDING would still work if a layer registered one.
        if (event.key === "Escape") disarm();

        const decision = handleKey(active, pending, event);
        if (decision.kind === "ignore") return;
        if (decision.kind === "clear") {
            // Released to the page: `g` then `x` should not be swallowed twice.
            disarm();
            return;
        }
        if (decision.kind === "pending") {
            // A claimed prefix must not also type its character somewhere.
            event.preventDefault();
            arm(decision.sequence);
            return;
        }
        // BEFORE run(): `/` focuses an input, and a default that has not been
        // prevented then types "/" into the input we just focused.
        event.preventDefault();
        disarm();
        decision.binding.run();
    },

    /**
     * Test-only. Module-level runes state is shared across a test file, so a
     * suite that registers layers outside a component must clear them.
     */
    reset(): void {
        layers = [];
        helpOpen = false;
        disarm();
    },
};
