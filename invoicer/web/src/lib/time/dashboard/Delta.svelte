<!-- A change against the previous period: an arrow and the signed figure,
     in text ink. Neutral on purpose: more hours is neither good nor bad on
     its own, so it never borrows a status colour (dataviz). The arrow is
     decoration; the label says it in words. -->
<script lang="ts">
    import type {Delta} from "./model.js";

    let {delta, testid}: {delta: Delta; testid?: string} = $props();

    const ARROW = {up: "▲", down: "▼", flat: "–"} as const;
</script>

<span
    class="inline-flex items-baseline gap-1 text-xs whitespace-nowrap text-base-content/70 tabular-nums"
    data-testid={testid}
    data-direction={delta.direction}
>
    <span aria-hidden="true" class="text-[0.6rem]">{ARROW[delta.direction]}</span>
    <span aria-hidden="true">{delta.text}</span>
    <span class="sr-only">{delta.label}</span>
</span>
