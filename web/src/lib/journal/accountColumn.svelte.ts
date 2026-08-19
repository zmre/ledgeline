// The width of ONE accounts cell, measured once for the whole table.
//
// The table is `table-fixed`, which makes every cell in a column exactly the
// same width — so the thirty-odd rows the virtualiser keeps alive do not each
// need measuring. That version is the one worth avoiding: a ResizeObserver per
// row, re-measuring on every scroll, to learn thirty copies of one number.
//
// One observer on the column's `<th>` reports the same content box every `<td>`
// in that column has: daisyUI's `table-sm` gives `th` and `td` the same
// `padding-inline` (0.75rem), and the rows override only `padding-block`
// (`py-1`). Every chip below reads the answer from here.
//
// This cannot oscillate. A wider column makes the chips render longer strings,
// but `table-layout: fixed` sizes columns from the `<colgroup>` and the table
// width alone — cell content is explicitly not consulted — so the text this
// feeds can never feed back into the measurement that produced it.

/** Content-box width in CSS px of one accounts cell; 0 means "nothing has measured it". */
class AccountColumn {
    width = $state(0);
}

export const accountColumn = new AccountColumn();

/**
 * Svelte action for the accounts `<th>`. Publishes the column's content width
 * for the chips to fit themselves to, and retracts it when the table goes away.
 */
export function measureAccountColumn(node: HTMLElement): {destroy(): void} | undefined {
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver((entries) => {
        const width = entries[0]?.contentRect?.width;
        if (typeof width !== "number" || !Number.isFinite(width) || width <= 0) return;
        // Nothing is published unless the column actually MOVED. Sub-pixel
        // noise would otherwise re-render every live row for no visible change,
        // and — more to the point — a width that keeps arriving unchanged stops
        // here instead of feeding another round of work, so no path through
        // this can sustain a resize loop even if some future layout does couple
        // the rows' height back to the column's width (a scrollbar appearing,
        // say). Cheap insurance on the one line that runs only in a browser.
        if (Math.abs(width - accountColumn.width) < 1) return;
        accountColumn.width = width;
    });
    try {
        observer.observe(node);
    } catch {
        // An engine that has the constructor but rejects the target is not
        // worth taking the table down for; the labels fall back to characters.
        return undefined;
    }
    return {
        destroy(): void {
            observer.disconnect();
            // The narrow card layout has no table and no `<th>`. Leaving a stale
            // desktop width behind would have its chips fitting themselves to a
            // cell that no longer exists.
            accountColumn.width = 0;
        },
    };
}
