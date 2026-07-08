<script lang="ts">
    // Problems drawer (WP-08): the drawer-side panel of the layout's daisyUI
    // drawer, listing problems grouped by rule. Clicking one closes the drawer,
    // widens the date filter if needed, navigates to the journal, and asks the
    // table (via problems.requestFocus) to scroll to and pulse the row.
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import type {Problem, Severity} from "$lib/checks/engine";
    import type {Transaction} from "$lib/domain/types";
    import {filters} from "$lib/stores/filters.svelte";
    import {journal} from "$lib/stores/journal.svelte";
    import {problems} from "$lib/stores/problems.svelte";

    const RULE_LABELS: Record<string, string> = {
        unbalanced: "Unbalanced",
        pending: "Pending",
        uncategorized: "Uncategorized",
        "missing-description": "Missing description",
        "future-date": "Future date",
    };

    const SEVERITY_BADGE: Record<Severity, string> = {error: "badge-error", warning: "badge-warning", info: "badge-info"};

    const txnByIndex = $derived.by(() => new Map(journal.txns.map((txn) => [txn.index, txn])));

    const groups = $derived.by(() => {
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- rebuilt wholesale inside $derived.by, never mutated afterwards
        const byRule = new Map<string, Problem[]>();
        for (const problem of problems.all) {
            const list = byRule.get(problem.rule);
            if (list === undefined) byRule.set(problem.rule, [problem]);
            else list.push(problem);
        }
        return [...byRule.entries()];
    });

    /** Widen the date filter just enough to include `txn` (accounts/query filters are left alone). */
    function widenDateRange(txn: Transaction): void {
        const current = filters.value;
        const from = current.from !== null && txn.date < current.from ? txn.date : current.from;
        const to = current.to !== null && txn.date > current.to ? txn.date : current.to;
        if (from !== current.from || to !== current.to) filters.setRange(from, to);
    }

    async function jumpTo(problem: Problem): Promise<void> {
        problems.drawerOpen = false;
        const txn = txnByIndex.get(problem.txnIndex);
        if (txn !== undefined) widenDateRange(txn);
        if (page.url.pathname !== resolve("/")) await goto(resolve("/"));
        problems.requestFocus(problem.txnIndex);
    }
</script>

<div class="drawer-side z-40">
    <label for="problems-drawer" aria-label="Close problems drawer" class="drawer-overlay"></label>
    <aside class="bg-base-200 text-base-content flex min-h-full w-80 max-w-[85vw] flex-col gap-3 p-4">
        <header class="flex items-center justify-between">
            <h2 class="text-base font-semibold">Problems</h2>
            <span class="text-base-content/60 text-sm">{problems.count === 1 ? "1 finding" : `${problems.count} findings`}</span>
        </header>

        {#if problems.count === 0}
            <p class="text-base-content/60 text-sm">No problems found. All checks pass.</p>
        {:else}
            {#each groups as [rule, list] (rule)}
                <section>
                    <h3 class="flex items-center gap-2 pb-1 text-sm font-medium">
                        <span class="badge badge-sm {SEVERITY_BADGE[list[0].severity]}">{list.length}</span>
                        {RULE_LABELS[rule] ?? rule}
                    </h3>
                    <ul class="flex flex-col gap-1">
                        {#each list as problem (problem.txnIndex + problem.message)}
                            {@const txn = txnByIndex.get(problem.txnIndex)}
                            <li>
                                <button type="button" class="hover:bg-base-300 w-full rounded-lg p-2 text-left" onclick={() => void jumpTo(problem)}>
                                    <span class="flex items-baseline gap-2">
                                        <span class="text-base-content/70 shrink-0 font-mono text-xs">{txn?.date ?? "—"}</span>
                                        <span class="truncate text-sm" title={txn?.description}>
                                            {txn === undefined || txn.description === "" ? "(no description)" : txn.description}
                                        </span>
                                    </span>
                                    <span class="text-base-content/60 block text-xs">{problem.message}</span>
                                </button>
                            </li>
                        {/each}
                    </ul>
                </section>
            {/each}
        {/if}
    </aside>
</div>
