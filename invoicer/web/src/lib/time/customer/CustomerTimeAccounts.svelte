<!-- A customer page's Time accounts (item 4): the time accounts linked to
     this customer (their own `customer:` tag, or one inherited from a
     parent), as a tree, with this month's hours and the unbilled hours of
     each (and below, as on the Accounts page). Each figure opens Entries on
     that account's time; the heading's links open all of the customer's
     time and the Accounts page.

     From `GET /api/time/accounts?month=` on mount and whenever a file
     changes. Nothing shows while it loads, before time tracking is set up,
     or when no account names the customer. -->
<script lang="ts">
    import {onMount} from "svelte";
    import AsyncSection from "#ui/components/AsyncSection.svelte";
    import type {DataView} from "#ui/ui/dataView.js";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import {EditorApiError} from "#lib/editorApi.js";
    import {localToday} from "#lib/invoices.js";
    import {fetchAccounts, type AccountsView} from "../api/accounts.js";
    import {duration} from "../dashboard/model.js";
    import {customerQuery, linkedAccounts, monthOf, monthQuery, unbilledQuery, type LinkedAccount} from "./customerTime.js";

    let {
        names,
        entriesHref,
        accountsHref,
        load = fetchAccounts,
        today = localToday,
    }: {
        /** What time accounts may call the customer: its ledger customer and its title. */
        names: string[];
        /** Entries with this query (no `?`), resolved. */
        entriesHref: (query: string) => string;
        /** Time → Accounts, resolved. */
        accountsHref: string;
        load?: (month: string) => Promise<AccountsView>;
        today?: () => string;
    } = $props();

    let rows = $state<LinkedAccount[] | null>(null);
    let error = $state<Error | null>(null);
    const view = $derived<DataView>(error !== null ? "error" : rows !== null ? "data" : "loading");

    let latest = 0;
    function refresh(): void {
        const mine = ++latest;
        const wanted = names;
        load(monthOf(today()))
            .then((answer) => {
                if (mine !== latest) return;
                rows = linkedAccounts(answer, wanted);
                error = null;
            })
            .catch((e: unknown) => {
                if (mine !== latest) return;
                // Before setup there are no time accounts: nothing to show.
                if (e instanceof EditorApiError && e.code === "not-set-up") rows = [];
                else if (rows === null) error = e instanceof Error ? e : new Error(String(e));
                else console.warn("customer time:", e);
            });
    }

    onMount(refresh);
    onFilesChanged(refresh);

    function retry(): void {
        error = null;
        refresh();
    }

    const customer = $derived(rows?.[0]?.customer ?? "");
</script>

{#if rows === null || rows.length > 0}
    <AsyncSection
        {view}
        value={rows}
        {error}
        testid="customer-time-error"
        label="the time accounts"
        loadingLabel="Loading time accounts"
        onRetry={retry}
        quietLoading
    >
        {#snippet children(list)}
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the page passes hrefs built with resolve() -->
            <section class="flex flex-col gap-2" aria-labelledby="customer-time-title" data-testid="customer-time-accounts">
                <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                    <h2 id="customer-time-title" class="text-sm font-semibold tracking-wide text-base-content/70 uppercase">Time accounts</h2>
                    <span class="flex gap-3 text-sm">
                        <a class="link link-hover" href={entriesHref(customerQuery(customer))} data-testid="customer-time-entries">All time</a>
                        <a class="link link-hover" href={accountsHref} data-testid="customer-time-accounts-link">Time accounts</a>
                    </span>
                </div>
                <div class="overflow-x-auto rounded-box border border-base-300">
                    <table class="table table-sm">
                        <thead>
                            <tr><th>Account</th><th class="text-right">This month</th><th class="text-right">Unbilled</th></tr>
                        </thead>
                        <tbody>
                            {#each list as row (row.account)}
                                <tr data-testid="customer-time-account" data-account={row.account}>
                                    <td class="min-w-0 break-all" style:padding-inline-start="{0.75 + row.indent * 1.25}rem">
                                        <a class="link link-hover" href={entriesHref(monthQuery(row.account))}>{row.account}</a>
                                        {#if row.inheritedFrom !== null}<span class="sr-only">, linked through {row.inheritedFrom}</span>{/if}
                                    </td>
                                    <td class="text-right whitespace-nowrap tabular-nums">
                                        <a class="link link-hover" href={entriesHref(monthQuery(row.account))} data-testid="customer-time-month"
                                            >{duration(row.month)}</a
                                        >
                                    </td>
                                    <td class="text-right whitespace-nowrap tabular-nums">
                                        {#if row.unbilled === 0}
                                            <span class="text-base-content/50" data-testid="customer-time-unbilled">—</span>
                                        {:else}
                                            <a class="link link-hover" href={entriesHref(unbilledQuery(row.account))} data-testid="customer-time-unbilled"
                                                >{duration(row.unbilled)}</a
                                            >
                                        {/if}
                                    </td>
                                </tr>
                            {/each}
                        </tbody>
                    </table>
                </div>
                <p class="text-xs text-base-content/60">Each account's hours include the accounts below it.</p>
            </section>
        {/snippet}
    </AsyncSection>
{/if}
