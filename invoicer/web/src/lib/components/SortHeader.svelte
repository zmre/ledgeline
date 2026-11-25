<!-- A sortable column header (invoicer PLAN 5b, S4): a button in the `<th>`,
     so Tab reaches it and Enter or Space sorts; the `<th>` carries
     `aria-sort` for screen readers, and the arrow is only for the eye. -->
<script lang="ts">
    import {ariaSort, type SortDir} from "#lib/listQuery.js";

    let {
        column,
        label,
        shown,
        onSort,
        align = "left",
    }: {
        /** The `sort` value this header sorts by. */
        column: string;
        label: string;
        /** The column the list is sorted by now, and which way. */
        shown: {sort: string; dir: SortDir};
        onSort: (column: string) => void;
        align?: "left" | "right";
    } = $props();

    const state = $derived(ariaSort(shown, column));
    const arrow = $derived(state === "ascending" ? "↑" : state === "descending" ? "↓" : "↕");
</script>

<th aria-sort={state} class={align === "right" ? "text-right" : ""}>
    <button
        type="button"
        class="-mx-1 inline-flex cursor-pointer items-center gap-1 rounded px-1 font-semibold whitespace-nowrap hover:text-base-content focus-visible:outline-2 focus-visible:outline-primary {align ===
        'right'
            ? 'flex-row-reverse'
            : ''}"
        onclick={() => onSort(column)}
        data-testid="sort-{column}"
        >{label}<span aria-hidden="true" class={state === "none" ? "text-base-content/30" : "text-base-content/80"}>{arrow}</span></button
    >
</th>
