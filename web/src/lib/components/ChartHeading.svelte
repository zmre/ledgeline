<!-- A chart's name, with an optional muted qualifier after a "·": the one
     heading style every chart and chart panel uses. -->
<script lang="ts">
    let {heading, note, class: className = ""}: {heading: string; note?: string; class?: string} = $props();
</script>

<h3 class={["text-xs font-semibold tracking-tight text-base-content/70", className]}>
    {heading}
    {#if note !== undefined}<span class="font-normal text-base-content/40">· {note}</span>{/if}
</h3>
