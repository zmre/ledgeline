<!-- The scenario list wired to the REAL store exactly as the Projections page
     wires it: the store's scenario in, `scenarioStore.commit` out. A harness,
     because `commit` replaces the scenario and a test's `render()` props are
     not reactive — the list has to read the store the way the page does. -->
<script lang="ts">
    import {untrack} from "svelte";
    import type {RenderProjectionBody} from "#lib/api/native.js";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import type {EntryFilter} from "#lib/projections/entries.js";
    import {scenarioStore} from "#lib/projections/scenarioStore.svelte.js";
    import type {AssetRow, ForecastDefaults} from "#lib/projections/types.js";
    import ScenarioList from "#lib/projections/ui/list/ScenarioList.svelte";

    let {
        accountNames,
        declared,
        styles,
        assetBalances = new Map(),
        defaults = null,
        seeded = false,
        readOnly = false,
        filter = "all",
        render,
    }: {
        accountNames: string[];
        declared: AccountClasses;
        styles: ReadonlyMap<string, AmountStyle>;
        assetBalances?: ReadonlyMap<string, AssetRow>;
        defaults?: ForecastDefaults | null;
        seeded?: boolean;
        readOnly?: boolean;
        filter?: EntryFilter;
        render?: (body: RenderProjectionBody) => Promise<string>;
    } = $props();

    // The initial chip only; the list owns it after.
    let shown = $state<EntryFilter>(untrack(() => filter));
</script>

<ScenarioList
    scenario={scenarioStore.scenario}
    {accountNames}
    {declared}
    commodity="$"
    stepDate="2027-04-01"
    {assetBalances}
    {defaults}
    {styles}
    {seeded}
    {readOnly}
    bind:filter={shown}
    {...render === undefined ? {} : {render}}
    onCommit={(next) => scenarioStore.commit(next)}
/>
