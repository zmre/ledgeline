<!-- The account tree (plan 46 T-dash; `hledger bal --tree`): every account
     with roll-ups at each level, only those with time this period (a
     footnote says how many were left out). Columns: hours, % of the period's total,
     billable hours and, with comparison on, the change against the
     previous period. Rows show to the filter bar's depth; each row's toggle
     opens it below that depth or closes it above, and "Reset" returns to
     the depth. Each account opens Entries for it and below.

     A plain table: the toggles are buttons with `aria-expanded`, the names
     links, all in tab order. Below `@md` the billable column folds away
     (the tiles carry the billable total), and below `@sm` the share. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {SvelteSet} from "svelte/reactivity";
    import InfoTip from "#ui/components/InfoTip.svelte";
    import Delta from "./Delta.svelte";
    import type {Command} from "./hledger.js";
    import {deltaOf, duration, percent, visibleRows, type TreeNode} from "./model.js";

    let {
        roots,
        hidden = 0,
        hiddenPrevious = 0,
        depth,
        total,
        compare,
        command,
        accountHref,
    }: {
        roots: TreeNode[];
        /** Accounts left out for having no time this period (`withTime`). */
        hidden?: number;
        /** Their previous period's seconds, said with comparison on. */
        hiddenPrevious?: number;
        depth: number;
        /** The period's total, for the share column. */
        total: number;
        compare: boolean;
        command: Command;
        accountHref: (account: string) => string;
    } = $props();

    const toggled = new SvelteSet<string>();
    // A new depth starts from it again.
    $effect(() => {
        void depth;
        untrack(() => toggled.clear());
    });

    const rows = $derived(visibleRows(roots, depth, toggled));
    const toggle = (account: string): void => {
        if (toggled.has(account)) toggled.delete(account);
        else toggled.add(account);
    };
    const id = $props.id();
</script>

<!-- eslint-disable svelte/no-navigation-without-resolve -- every href is built on the page's resolved Entries base -->
<section class="flex min-w-0 flex-col gap-2 rounded-box border border-base-300 bg-base-100 p-4" aria-labelledby="{id}-title" data-testid="time-dash-tree">
    <div class="flex items-center justify-between gap-2">
        <h2 id="{id}-title" class="flex items-center gap-1.5 text-sm font-medium text-base-content/70">
            Accounts
            <InfoTip
                label="the account tree"
                text={command.caveat === undefined ? "Reproduce it with hledger." : `Reproduce it with hledger. ${command.caveat}`}
                formula={command.text}
            />
        </h2>
        {#if toggled.size > 0}
            <button type="button" class="btn btn-ghost btn-xs" onclick={() => toggled.clear()} data-testid="time-dash-tree-reset">Reset to depth {depth}</button
            >
        {/if}
    </div>
    {#if roots.length === 0}
        <p class="py-4 text-sm text-base-content/60">No time in this period.</p>
    {:else}
        <div class="@container overflow-x-auto">
            <table class="table table-sm">
                <thead>
                    <tr>
                        <th scope="col">Account</th>
                        <th scope="col" class="text-right">Hours</th>
                        <th scope="col" class="hidden text-right @sm:table-cell">Share</th>
                        <th scope="col" class="hidden text-right @md:table-cell">Billable</th>
                        {#if compare}<th scope="col" class="text-right">Change</th>{/if}
                    </tr>
                </thead>
                <tbody>
                    {#each rows as row (row.node.account)}
                        {@const n = row.node}
                        <tr data-testid="time-dash-tree-row" data-account={n.account} data-depth={n.depth}>
                            <th scope="row" class="font-normal">
                                <span class="flex items-center gap-1" style="padding-left: {n.depth * 1.25}rem">
                                    {#if row.expandable}
                                        <button
                                            type="button"
                                            class="btn btn-square btn-ghost btn-xs"
                                            aria-expanded={row.expanded}
                                            aria-label="{row.expanded ? 'Collapse' : 'Expand'} {n.account}"
                                            onclick={() => toggle(n.account)}
                                            data-testid="time-dash-tree-toggle"
                                        >
                                            <span aria-hidden="true">{row.expanded ? "▾" : "▸"}</span>
                                        </button>
                                    {:else}
                                        <span class="inline-block w-6 shrink-0" aria-hidden="true"></span>
                                    {/if}
                                    <a class={["link truncate link-hover", n.depth === 0 && "font-medium"]} href={accountHref(n.account)} title={n.account}
                                        >{n.name}</a
                                    >
                                </span>
                            </th>
                            <td class={["text-right whitespace-nowrap tabular-nums", n.seconds === 0 && "text-base-content/50"]}>{duration(n.seconds)}</td>
                            <td class="hidden text-right whitespace-nowrap text-base-content/70 tabular-nums @sm:table-cell"
                                >{total > 0 ? percent(n.seconds / total) : "—"}</td
                            >
                            <td class="hidden text-right whitespace-nowrap text-base-content/70 tabular-nums @md:table-cell"
                                >{n.billable === 0 ? "—" : duration(n.billable)}</td
                            >
                            {#if compare}<td class="text-right whitespace-nowrap"><Delta delta={deltaOf(n.seconds, n.previous, "duration")} /></td>{/if}
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {/if}
    {#if hidden > 0}
        <p class="text-xs text-base-content/60" data-testid="time-dash-tree-hidden">
            Not shown: {hidden === 1 ? "1 account" : `${hidden} accounts`} with no time in this period{compare && hiddenPrevious > 0
                ? ` (${duration(hiddenPrevious)} in the previous one)`
                : ""}.
        </p>
    {/if}
</section>
