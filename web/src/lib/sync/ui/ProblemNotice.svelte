<!-- One bank sync problem (plan 43 §6): who has to act, the plain sentence,
     since when, one primary action, and a details disclosure with the raw code
     and a link to its docs section.

     An "open-url" action is a real link (`target="_blank"`, `noopener`), so
     middle-click and copy-link work; every other action is a button that hands
     its effect to the host. -->
<script lang="ts">
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {docUrl, problemButtons, whoLabel, type ProblemEffect, type ProblemScope} from "../problemAction";
    import {dataThroughLine} from "../statusLine";
    import type {WireProblem} from "../types";

    let {
        problem,
        scope,
        onEffect,
        busy = false,
        compact = false,
    }: {
        problem: WireProblem;
        scope: ProblemScope;
        onEffect: (effect: ProblemEffect) => void;
        /** A sync is running: the buttons that would start another wait. */
        busy?: boolean;
        /** The Problems drawer's narrow list: no box, smaller text. */
        compact?: boolean;
    } = $props();

    const buttons = $derived(problemButtons(problem, scope));
    const tone = $derived(problem.severity === "error" ? "alert-error" : problem.severity === "warning" ? "alert-warning" : "alert-info");
</script>

{#snippet action(label: string, effect: ProblemEffect, primary: boolean)}
    {#if effect.type === "url"}
        <a class="btn btn-xs {primary ? '' : 'btn-ghost'}" href={effect.url} target="_blank" rel="noopener noreferrer external">{label}</a>
    {:else}
        <button type="button" class="btn btn-xs {primary ? '' : 'btn-ghost'}" disabled={busy && effect.type === "sync"} onclick={() => onEffect(effect)}>
            {#if busy && effect.type === "sync"}<span class="loading loading-xs loading-spinner"></span>{/if}
            {label}
        </button>
    {/if}
{/snippet}

<div
    class={compact ? "flex flex-col gap-1 rounded-lg p-2 text-xs" : `alert flex flex-col items-start gap-1 alert-soft px-3 py-2 text-sm ${tone}`}
    role={problem.severity === "error" ? "alert" : "status"}
    data-testid="sync-problem"
    data-code={problem.code}
>
    <span class="text-xs font-medium opacity-70">{whoLabel(problem.who)}</span>
    <span>
        {problem.severity === "warning" ? "⚠ " : ""}{problem.message}
        {#if problem.dataThrough !== null}
            <span class="whitespace-nowrap opacity-80">{dataThroughLine(problem.dataThrough, localToday())}.</span>
        {/if}
    </span>
    {#if buttons.primary !== null || buttons.secondary !== null}
        <div class="flex flex-wrap items-center gap-2">
            {#if buttons.primary !== null}{@render action(buttons.primary.label, buttons.primary.effect, true)}{/if}
            {#if buttons.secondary !== null}{@render action(buttons.secondary.label, buttons.secondary.effect, false)}{/if}
        </div>
    {/if}
    {#if problem.detail !== null || problem.docAnchor !== ""}
        <details class="text-xs opacity-70">
            <summary class="cursor-pointer">Details</summary>
            {#if problem.detail !== null}
                <code class="block font-mono break-all whitespace-pre-wrap">{problem.detail}</code>
            {/if}
            {#if problem.docAnchor !== ""}
                <a class="link" href={docUrl(problem.docAnchor)} target="_blank" rel="noopener noreferrer external">What this means and how to fix it ↗</a>
            {/if}
        </details>
    {/if}
</div>
