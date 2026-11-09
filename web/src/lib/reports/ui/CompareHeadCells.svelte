<!-- The Δ and Δ% column headings of a projection comparison (plans/34), in
     place of a statement's Total heading. -->
<script lang="ts">
    let {cellClass = "text-right font-normal"}: {cellClass?: string} = $props();
</script>

<th class={cellClass} data-column="delta-heading" title="Actual minus as projected, over the whole window">Δ</th>
<th class={cellClass} data-column="delta-pct-heading" title="The difference as a share of the projected figure">Δ%</th>
