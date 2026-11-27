<!-- The Problems drawer's "Bank sync" group (plan 43 §4.1): the sync status's
     problems beside the journal checks, each with its fix. A fix that opens
     the wizard or an account's drawer closes this drawer and goes to
     Imports → Accounts, which takes the request. -->
<script lang="ts">
    import {problems as journalProblems} from "#lib/stores/problems.svelte.js";
    import type {ProblemEffect} from "../problemAction";
    import {syncStore} from "../syncStore.svelte";
    import {runEffect} from "./effects";
    import ProblemNotice from "./ProblemNotice.svelte";

    const BADGE = {error: "badge-error", warning: "badge-warning", info: "badge-info"} as const;

    function onEffect(effect: ProblemEffect): void {
        if (effect.type !== "url" && effect.type !== "sync") journalProblems.drawerOpen = false;
        void runEffect(effect);
    }
</script>

{#if syncStore.problems.length > 0}
    <section data-testid="sync-problems-group">
        <h3 class="flex items-center gap-2 pb-1 text-sm font-medium">
            <span class="badge badge-sm {BADGE[syncStore.maxSeverity ?? 'warning']}">{syncStore.problems.length}</span>
            Bank sync
        </h3>
        <ul class="flex flex-col gap-1">
            {#each syncStore.problems as placed (placed.key)}
                <li class="rounded-lg bg-base-100/40">
                    <span class="block px-2 pt-2 text-xs font-medium text-muted">{placed.where}</span>
                    <ProblemNotice problem={placed.problem} scope={placed.scope} {onEffect} busy={syncStore.running} compact />
                </li>
            {/each}
        </ul>
    </section>
{/if}
