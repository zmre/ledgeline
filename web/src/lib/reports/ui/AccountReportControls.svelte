<!-- The Accounts report's own controls, on Reports and on Projections: which
     accounts to show (the journal filter's tree dropdown, fed the accounts this
     report has rows for) and whether empty accounts are rows too. The caller
     holds both values and applies the selection with `filterAccountReport`, so
     its export narrows with the table. -->
<script lang="ts">
    import AccountTreeSelect from "#lib/filters/AccountTreeSelect.svelte";
    import {toggleSubtreeRoot} from "#lib/filters/treeSelect.js";
    import type {AccountReport} from "#lib/reports/types.js";
    import {reportAccounts} from "./accountReportRows";

    let {
        report,
        selected = $bindable(),
        empty = $bindable(),
    }: {
        /** The UNFILTERED report: the dropdown offers every account it has a row for. Null while it is being read. */
        report: AccountReport | null;
        /** Selected subtree roots; empty for every account. */
        selected: ReadonlySet<string>;
        empty: boolean;
    } = $props();

    const accountNames = $derived(report === null ? [] : reportAccounts(report));
</script>

<div class="flex flex-wrap items-center gap-x-4 gap-y-2" data-testid="account-report-controls">
    <AccountTreeSelect {accountNames} {selected} onToggle={(name) => (selected = toggleSubtreeRoot(selected, name))} onClear={() => (selected = new Set())} />
    <label class="label cursor-pointer gap-2 text-sm">
        <input type="checkbox" class="checkbox checkbox-sm" bind:checked={empty} data-testid="accounts-empty" />
        Show empty accounts
    </label>
</div>
