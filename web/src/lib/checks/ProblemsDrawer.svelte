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
        // Engine-computed (see CheckContext.diagnostics). "unbalanced" above is
        // shared: the engine's finding and the local rule land in one group.
        assertion: "Balance assertion",
        pending: "Pending",
        uncategorized: "Uncategorized",
        "missing-description": "Missing description",
        "future-date": "Future date",
        // Engine-computed, and the only rule anchored to an ACCOUNT rather than
        // a transaction: an `account` directive whose type:/issection:/bsterm:/
        // holdings:/valuation: value is outside its closed vocabulary. The tag
        // is ignored and the report falls back, so this is a warning.
        "account-tag": "Unknown account tag value",
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

    /**
     * Jump to the transaction a problem flags.
     *
     * Only ever called for an ANCHORED problem — an unanchored one renders as
     * plain text with no click target, because an `account` directive has no row
     * to scroll to and faking a jump to some other row would be worse than not
     * offering one. The null guard is belt-and-braces for that contract.
     */
    async function jumpTo(problem: Problem): Promise<void> {
        if (problem.txnIndex === null) return;
        problems.drawerOpen = false;
        const txn = txnByIndex.get(problem.txnIndex);
        if (txn !== undefined) widenDateRange(txn);
        if (page.url.pathname !== resolve("/")) await goto(resolve("/"));
        problems.requestFocus(problem.txnIndex);
    }

    /**
     * Svelte `{#each}` key. Two findings can share a message only if they share
     * an anchor, and Svelte throws on a duplicate key, so both halves are in it.
     *
     * `\u0000` as the separator, written as the ESCAPE and not as a literal NUL
     * byte: a raw one makes git treat the whole file as binary, which silently
     * costs every future reviewer the diff. It cannot occur in an account name
     * or a message, which is why it is the separator.
     */
    const problemKey = (problem: Problem): string => `${problem.account ?? problem.txnIndex}\u0000${problem.message}`;
</script>

<!-- Engine diagnostics are hledger-style: several lines whose numbers are
     column-aligned. `whitespace-pre` + mono keeps that alignment (a wrap would
     destroy it) and the narrow drawer scrolls sideways instead of truncating.
     Single-line messages keep the original wrapping treatment. -->
{#snippet messageBody(problem: Problem)}
    {#if problem.message.includes("\n")}
        <span class="block overflow-x-auto font-mono text-xs whitespace-pre text-base-content/60">{problem.message}</span>
    {:else}
        <span class="block text-xs text-base-content/60">{problem.message}</span>
    {/if}
{/snippet}

<!-- The `{#if problems.drawerOpen}` is load-bearing, not tidiness. daisyUI hides a
     closed drawer with `visibility: hidden`, NOT `display: none`, and the panel
     sits under a permanent `will-change: transform` layer — so without the guard
     every finding is built, laid out and composited on every page of the app,
     for a panel nobody has opened. On a large journal that is ~10 DOM nodes per
     finding (21,429 findings ≈ 215,000 nodes), rebuilt on every journal state
     swap. The `<aside>` itself stays mounted unconditionally: it is the element
     daisyUI slides in, so keeping it is what preserves the open transition. -->
<div class="drawer-side z-40">
    <label for="problems-drawer" aria-label="Close problems drawer" class="drawer-overlay"></label>
    <aside class="flex min-h-full w-80 max-w-[85vw] flex-col gap-3 bg-base-200 p-4 text-base-content">
        {#if problems.drawerOpen}
            <header class="flex items-center justify-between">
                <h2 class="text-base font-semibold">Problems</h2>
                <span class="text-sm text-base-content/60">{problems.count === 1 ? "1 finding" : `${problems.count} findings`}</span>
            </header>

            {#if problems.count === 0}
                <p class="text-sm text-base-content/60">No problems found. All checks pass.</p>
            {:else}
                {#each groups as [rule, list] (rule)}
                    <section>
                        <h3 class="flex items-center gap-2 pb-1 text-sm font-medium">
                            <span class="badge badge-sm {SEVERITY_BADGE[list[0].severity]}">{list.length}</span>
                            {RULE_LABELS[rule] ?? rule}
                        </h3>
                        <ul class="flex flex-col gap-1">
                            {#each list as problem (problemKey(problem))}
                                <li>
                                    {#if problem.txnIndex === null}
                                        <!-- Unanchored: a finding about an `account` DIRECTIVE. Rendered as
                                         plain text, deliberately NOT a button — there is no transaction to
                                         scroll to, and a click target that did nothing (or jumped somewhere
                                         arbitrary) would be worse than none. The account name takes the slot
                                         the date/description occupies for a transaction finding, so the
                                         column still answers "what is this about?". -->
                                        <div class="w-full rounded-lg p-2 text-left">
                                            <span class="flex items-baseline gap-2">
                                                <span class="truncate font-mono text-xs text-base-content/70" title={problem.account}
                                                    >{problem.account ?? "—"}</span
                                                >
                                            </span>
                                            {@render messageBody(problem)}
                                        </div>
                                    {:else}
                                        {@const txn = txnByIndex.get(problem.txnIndex)}
                                        <button type="button" class="w-full rounded-lg p-2 text-left hover:bg-base-300" onclick={() => void jumpTo(problem)}>
                                            <span class="flex items-baseline gap-2">
                                                <span class="shrink-0 font-mono text-xs text-base-content/70">{txn?.date ?? "—"}</span>
                                                <span class="truncate text-sm" title={txn?.description}>
                                                    {txn === undefined || txn.description === "" ? "(no description)" : txn.description}
                                                </span>
                                            </span>
                                            {@render messageBody(problem)}
                                        </button>
                                    {/if}
                                </li>
                            {/each}
                        </ul>
                    </section>
                {/each}
            {/if}
        {/if}
    </aside>
</div>
