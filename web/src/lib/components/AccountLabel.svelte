<script lang="ts">
    // One account name, shortened from the LEFT when it does not fit.
    //
    // # The bug
    //
    // Account names are hierarchies whose meaning is at the end. Every chip in
    // the journal list was a `truncate`, so `expenses:auto:maintenance` in a
    // narrow accounts column read `expenses:auto:ma…` — the segment the reader
    // already knows, with the one they are looking for cut off. Two accounts
    // that differ only in their leaf rendered identically.
    //
    // # Two mechanisms, in this order
    //
    // 1. `fitAccount` (pure, tested) spends the ANCESTORS:
    //    `expenses:household:repairs:plumbing` → `exp:household:repairs:plumbing`.
    //    Every segment survives, so the path keeps its shape and depth.
    // 2. Whatever is still too wide is clipped by CSS at the LEFT edge, so the
    //    leaf stays on screen: `…s:auto:maintenance`, then `…aintenance`.
    //
    // The second is what makes this correct at ANY width. The first only earns
    // its place when it fires no sooner than it must, and THAT is what `maxWidth`
    // is for.
    //
    // # Why this measures, having once refused to
    //
    // The first version of this component abbreviated to a fixed thirty
    // CHARACTERS. Characters are not a unit of width in a proportional font, and
    // the chip is a share of a table column that moves with the window and with
    // which columns are switched on, so one number could only be right at one
    // size — and being tuned for the widest case, it was too small everywhere
    // else. Names were cut to thirty characters inside chips with room for
    // nearer forty, and because a badge is `width: fit-content` the slack did
    // not go to the text: it pooled as dead space at the end of the column.
    //
    // What made measuring look expensive was assuming it had to happen per row.
    // It does not. `table-fixed` gives every cell in a column one width, so ONE
    // ResizeObserver on the column header serves all thirty live rows
    // (accountColumn.svelte.ts), and the string measuring is `canvas.measureText`,
    // which reads font metrics without touching layout (textWidth.ts). Neither
    // reads geometry per row, so neither can thrash.
    //
    // Where no measurement exists — the card layout, server rendering, the first
    // frame — `budget` still applies as a coarse floor. It is a fallback now, not
    // the design.
    //
    // # Why `dir="rtl"`
    //
    // `text-overflow: ellipsis` puts its ellipsis at the END of the line, and
    // the end of an RTL line is on the left. `text-align: left` then keeps the
    // text flush left as before, so the only visible change is which end the
    // ellipsis eats. The `<bdi>` is not decoration: an RTL line reorders
    // neutral characters at its edges, which would move a trailing `:` or `)`
    // to the front of the name. Isolating the text pins its own order and
    // leaves only the ellipsis in RTL's hands.
    //
    // # What a screen reader gets
    //
    // The full name, always: when the visible text is an abbreviation it is
    // `aria-hidden` and the real name rides along `sr-only` (the same split
    // RenameList makes). `title` is the full name too unless the caller passes
    // its own — the journal chips prefix theirs with "Edit category ·".
    import {abbreviateAccount, fitAccount} from "$lib/domain/accounts";
    import {chipMeasurer} from "./textWidth";

    let {
        name,
        title,
        budget,
        maxWidth,
    }: {
        name: string;
        /** Tooltip text. Defaults to the full account name; a caller replacing it must still say the whole name. */
        title?: string;
        /** Fallback: characters to fit when no `maxWidth` is known (default `ACCOUNT_LABEL_BUDGET`). */
        budget?: number;
        /** CSS px of room the text actually has. Preferred over `budget` whenever the caller has measured it. */
        maxWidth?: number;
    } = $props();

    // Measured pixels when both the caller and the engine can supply them,
    // characters when either cannot. `budget` being undefined falls through to
    // the helper's own default rather than being special-cased here.
    const display = $derived.by(() => {
        const measure = maxWidth === undefined ? null : chipMeasurer();
        if (measure === null || maxWidth === undefined) return abbreviateAccount(name, budget);
        return fitAccount(name, maxWidth, measure);
    });
</script>

<span class="truncate text-left" dir="rtl" title={title ?? name}>
    {#if display === name}
        <bdi>{name}</bdi>
    {:else}
        <span class="sr-only">{name}</span><bdi aria-hidden="true">{display}</bdi>
    {/if}
</span>
