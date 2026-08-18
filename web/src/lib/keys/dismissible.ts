// Escape-to-close, focus trap, and focus restore, as one `use:` action.
//
// Six overlays in this app currently roll their own dismissal and NONE of them
// restores focus — open the column menu, close it, and focus is on <body>. Only
// ColumnMenu handles Escape and outside-click at all; ServerSetupModal has
// neither. This is the shared piece those should converge on.
//
// The topmost-only rule is what makes nested overlays behave WITHOUT
// `stopPropagation` (which this codebase deliberately never uses): every ACTIVE
// instance pushes onto a LIFO stack, and only the last one responds to Escape.

import {FOCUSABLE} from "./target";

export interface DismissibleOptions {
    onDismiss: () => void;
    /**
     * Whether the overlay is currently open. Defaults to true, for the common
     * case of an `{#if}`-gated overlay whose mount IS its opening.
     *
     * The flag exists because some overlays here are always mounted (the
     * transaction popup) and some hosts cannot be conditionally decorated (a
     * `<details>` needs the action on the element that owns `open`). Without it
     * those would hold the top of the stack permanently and swallow Escape from
     * everything below — and would keep an idle document listener, which is
     * exactly what ColumnMenu's "attached only while open" comment forbids.
     */
    active?: boolean;
    /** Trap Tab inside and focus the first focusable on open. Screen-owning overlays want this; dropdowns do not. */
    trap?: boolean;
    /** Dismiss on a pointerdown outside the node. Dropdowns want this; modals use their backdrop button instead. */
    outside?: boolean;
}

const stack: HTMLElement[] = [];

function focusables(node: HTMLElement): HTMLElement[] {
    return [...node.querySelectorAll<HTMLElement>(FOCUSABLE)];
}

/**
 * Focus the first candidate that will actually take focus, falling back to the
 * container itself.
 *
 * Deliberately behavioural rather than a visibility filter: `.focus()` on a
 * `display:none` element is a silent no-op, so "did focus move?" is the real
 * question, and asking it directly works in both a browser and jsdom. An
 * `offsetParent`/`checkVisibility` filter would be a proxy for the same
 * question that jsdom — which has no layout engine — always answers "hidden",
 * making the trap untestable and, worse, wrong in tests only.
 */
function focusFirst(node: HTMLElement): void {
    for (const candidate of focusables(node)) {
        candidate.focus();
        if (document.activeElement === candidate) return;
    }
    // Nothing took focus. Set `tabindex` programmatically rather than in markup,
    // so the app's markup keeps its single `tabindex` and stays clean under
    // svelte-check.
    node.tabIndex = -1;
    node.focus();
}

export function dismissible(node: HTMLElement, options: DismissibleOptions) {
    let current = options;
    let attached = false;
    let opener: HTMLElement | null = null;

    function isTopmost(): boolean {
        return stack[stack.length - 1] === node;
    }

    function onKeyDown(event: KeyboardEvent): void {
        if (!isTopmost()) return;
        if (event.key === "Escape") {
            event.preventDefault();
            current.onDismiss();
            return;
        }
        if (event.key !== "Tab" || current.trap !== true) return;
        const items = focusables(node);
        if (items.length === 0) {
            // Nothing to tab to — keep focus in the overlay rather than letting
            // it escape to the page behind, which a trap exists to prevent.
            event.preventDefault();
            return;
        }
        const first = items[0];
        const last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) {
            event.preventDefault();
            last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
            event.preventDefault();
            first.focus();
        }
    }

    function onPointerDown(event: PointerEvent): void {
        if (!isTopmost() || current.outside !== true) return;
        if (!node.contains(event.target as Node)) current.onDismiss();
    }

    function attach(): void {
        if (attached) return;
        attached = true;
        // Captured before we move focus anywhere, so closing returns the user to
        // whatever opened this rather than dumping them on <body>.
        opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        stack.push(node);
        document.addEventListener("keydown", onKeyDown);
        document.addEventListener("pointerdown", onPointerDown, true);
        if (current.trap === true) focusFirst(node);
    }

    function detach(): void {
        if (!attached) return;
        attached = false;
        document.removeEventListener("keydown", onKeyDown);
        document.removeEventListener("pointerdown", onPointerDown, true);
        const at = stack.indexOf(node);
        if (at !== -1) stack.splice(at, 1);
        if (opener !== null && opener.isConnected) opener.focus();
        opener = null;
    }

    if (current.active !== false) attach();

    return {
        update(next: DismissibleOptions): void {
            current = next;
            if (next.active !== false) attach();
            else detach();
        },
        destroy(): void {
            detach();
        },
    };
}
