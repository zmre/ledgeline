// How much room the text inside a journal account chip actually gets.
//
// These numbers are the CSS in AccountsCell.svelte written down, and they are
// only ever used to decide WHICH STRING to render — never to set a width. The
// browser still lays the chips out, so being a pixel or two out here costs at
// worst one character of abbreviation that was not needed, and never a broken
// layout. Anything that moves the classes in AccountsCell should move these
// with them; chipGeometry.test.ts pins each one to the class it came from.

import {fitAccount, shareWidths, type MeasureText} from "$lib/domain/accounts";

/**
 * Everything a chip spends before its text starts, per chip.
 *
 * daisyUI `badge-sm` sets `--size: calc(0.25rem * 5)` = 20px and
 * `padding-inline: calc(var(--size) / 2 - var(--border))` = 9px a side, on top
 * of the 1px `--border` itself: (9 + 1) * 2.
 */
export const CHIP_CHROME_PX = 20;

/** The two `gap-1` gaps either side of the arrow in a from→to row: 4px each. */
export const FLOW_GAP_PX = 8;

/** The `→` itself, at the 0.75rem font size `table-sm` gives the row. */
export const FLOW_ARROW_PX = 12;

/**
 * Text room for each chip of a `source → dest` cell `cellWidth` px wide.
 *
 * The two chips SHARE the cell. That is the whole point: capping each at 45%
 * independently meant a short source could not lend its slack to a long
 * destination, so an `assets:bank:checking → expenses:household:repairs:plumbing`
 * row left the tail of the column empty no matter how the budget was tuned.
 */
export function flowChipRooms(names: readonly string[], cellWidth: number, measure: MeasureText): number[] {
    return settleRooms(names, cellWidth - FLOW_ARROW_PX - FLOW_GAP_PX, measure);
}

/**
 * Text room for each chip of an N-way split, which is a `flex-wrap` list.
 *
 * Wrapping means a chip that cannot share a line gets a line of its own, so the
 * width worth fitting to is the whole cell: abbreviate only a name that will not
 * fit even then. Chips short enough to sit together already fit by definition.
 */
export function splitChipRooms(names: readonly string[], cellWidth: number): number[] {
    return names.map(() => cellWidth - CHIP_CHROME_PX);
}

// Passes of hand-back below. Each one buys at least one chip one more rung, so
// a two-chip row settles in a few; the bound is only here so that a deep enough
// name cannot spin.
const SETTLE_PASSES = 8;

// Pixels below which a leftover is not worth another pass.
const CRUMB_PX = 0.5;

/**
 * Divide `room` between `names`, then give back what the abbreviations did not
 * spend — repeatedly, until nothing moves.
 *
 * The second half is not a refinement, it is the difference between this
 * working and not. Abbreviation moves in RUNGS (a whole segment drops to three
 * characters at a time), so a chip handed 180px will commonly render a 150px
 * rung, and because a badge is `width: fit-content` those 30px do not go to its
 * neighbour — they evaporate exactly the way the old fixed budget's did. A first
 * cut of this fix measured correctly and still lost ground at some widths for
 * precisely that reason. So each pass asks what the chosen strings ACTUALLY use,
 * pools the difference, and offers it to a chip that is still shortened; a chip
 * already showing its full name is pinned at its natural width and lends the
 * rest.
 *
 * The leftover goes to ONE chip at a time rather than being split evenly, and
 * that detail matters: a rung is the smallest thing there is to buy, so half a
 * rung each can be too little for either chip to move while the whole of it
 * would have bought one of them a segment.
 *
 * It stops when no single chip could take the entire leftover and show any more
 * of its name than it already does — which is exactly the property
 * chipGeometry.test.ts asserts, and exactly what "the label fills the column"
 * means when it cannot be seen.
 */
function settleRooms(names: readonly string[], room: number, measure: MeasureText): number[] {
    const boxes = names.map((name) => measure(name) + CHIP_CHROME_PX);
    let shares = shareWidths(boxes, room);
    for (let pass = 0; pass < SETTLE_PASSES; pass++) {
        const labels = names.map((name, i) => fitAccount(name, shares[i] - CHIP_CHROME_PX, measure));
        // A last-resort rung can still overflow its share; CSS clips it there,
        // so what it OCCUPIES is the share and not the string's own width.
        const used = labels.map((label, i) => Math.min(measure(label) + CHIP_CHROME_PX, shares[i]));
        const slack = room - used.reduce((sum, width) => sum + width, 0);
        if (slack <= CRUMB_PX) break;
        const winner = names.findIndex((name, i) => labels[i] !== name && fitAccount(name, used[i] - CHIP_CHROME_PX + slack, measure) !== labels[i]);
        if (winner === -1) break;
        // Re-based on what is really used, so the shares still total `room`.
        shares = used.slice();
        shares[winner] = used[winner] + slack;
    }
    return shares.map((share) => share - CHIP_CHROME_PX);
}
