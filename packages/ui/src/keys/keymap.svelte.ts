// The keymap store, shared by Ledgeline and the invoicer (one per SPA: the
// state is module-level, and each app bundles its own copy). The layer stack, the armed chord prefix, the help sheet's
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
import {fieldBindings, handleKey, helpSections, inScope, resolveBindings, type HelpSection} from "./dispatch";
import {isAltLetter} from "./chord";
import {isTypingTarget, type TargetLike} from "./target";
import {keyTrace, recordKey} from "./trace";
import {CHORD_TIMEOUT_MS, type KeyGroup, type Layer, type RegisteredLayer, type ResolvedBinding} from "./types";

let layers = $state<RegisteredLayer[]>([]);
let pending = $state("");
let helpOpen = $state(false);
// The app's help-sheet section order (`setGroupOrder`). Empty: sections in
// the order their first binding resolves.
let groupOrder = $state<readonly KeyGroup[]>([]);

let seq = 0;
let pendingTimer: ReturnType<typeof setTimeout> | undefined;

const active = $derived.by(() => resolveBindings(layers));
const help = $derived.by(() => helpSections(active, groupOrder));

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
    /**
     * The app's help-sheet sections, in the order the `?` sheet shows them.
     * Called once at startup by each app's root layout; a section it leaves
     * out still shows, after these.
     */
    setGroupOrder(order: readonly KeyGroup[]): void {
        groupOrder = [...order];
    },
    /** Drop any armed chord prefix. Also bound to window blur, for Cmd-Tab. */
    disarm,

    /** The one keydown entry point. Wired in `+layout.svelte`. */
    handle(event: KeyboardEvent): void {
        // e2e only: see trace.ts. Undefined in the app, so nothing is recorded.
        const trace = keyTrace();
        // A local handler already claimed it (see the file header).
        if (event.defaultPrevented) {
            if (trace) recordKey(trace, event, "prevented");
            return;
        }
        // Mid-IME composition, and the dead-key half of an accented character
        // (Option-e on a US layout in the WKWebView) — neither is a keystroke yet.
        // An Alt+letter whose Option character is a dead key (Option+I on a US
        // Mac) is still a chord: `chordToken` reads it from the physical key.
        if (event.isComposing || (event.key === "Dead" && !isAltLetter(event)) || event.key === "Unidentified") {
            if (trace) recordKey(trace, event, "composing");
            return;
        }
        // Focus is in a field: the app is not listening, Escape included.
        // Escape inside a field belongs to that field, which is the shape the
        // four pre-existing handlers already have. The ONE carve-out is a
        // binding marked `always` (zoom): a Ctrl/Cmd chord that types nothing,
        // so a field cannot want it, and browser zoom works from a field too.
        // It is a property of the binding, not a list here, so it cannot drift.
        // `fields` is the same carve-out for an editor's own chords, minus
        // the reach through a modal layer (see `Binding.fields`).
        // A binding scoped to an element (`within`) only answers keys aimed inside it.
        const candidates = inScope(active, event.target);
        if (isTypingTarget(event.target as TargetLike | null)) {
            disarm();
            const decision = handleKey(fieldBindings(candidates), "", event);
            if (trace) recordKey(trace, event, "field", {layers, cached: active, fresh: decision, decision});
            if (decision.kind === "run") {
                event.preventDefault();
                decision.binding.run();
            }
            return;
        }
        // Escape always disarms a half-typed chord, then falls through so an
        // Escape BINDING would still work if a layer registered one.
        if (event.key === "Escape") disarm();

        const decision = handleKey(candidates, pending, event);
        if (trace)
            recordKey(trace, event, decision.kind, {
                layers,
                cached: active,
                fresh: handleKey(inScope(resolveBindings(layers), event.target), pending, event),
                decision,
            });
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
