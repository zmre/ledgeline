// Clickable list rows (Phase 3 feedback F4; the invoice, customer and
// template lists). A row isn't made a link: it can't wrap its one real link
// without nesting interactive elements. The row's link (`a[data-row-link]`)
// is what the keyboard reaches with Tab and opens with Enter, and what a
// screen reader reads; a click anywhere else on the row is the pointer's
// shortcut to it.

/** What a click on a control or label does itself, so the row leaves it alone. */
const CONTROLS = "a, button, input, select, textarea, label, summary";

/**
 * Open a row from a click on it: `open` when given, else the row's own link
 * is clicked (so the link's tab rules apply). Nothing happens for a click on
 * a control in the row, or one that ends a text selection.
 */
export function rowClick(event: MouseEvent, open?: () => void): void {
    const target = event.target as Element | null;
    if (target?.closest(CONTROLS) ?? false) return;
    if ((window.getSelection?.()?.toString() ?? "") !== "") return;
    if (open !== undefined) {
        open();
        return;
    }
    (event.currentTarget as HTMLElement | null)?.querySelector<HTMLAnchorElement>("a[data-row-link]")?.click();
}
