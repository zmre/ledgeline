<!-- The dialog's fields for an automatic posting (`= QUERY` block): the
     existing `AutoRuleEditor` form, unchanged (plan 30). A rule the form
     cannot state is shown as its text, locked, with the reason — it is kept,
     applied and saved exactly as written. An unscoped hand-written rule offers
     one click to say "forecasts only" in the file. -->
<script lang="ts">
    import AutoRuleEditor from "#lib/autoRules/AutoRuleEditor.svelte";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {autoLockedReason} from "../../../entryText";
    import {ruleSample} from "../../../scenarioModel";
    import type {AutoRule, Scenario} from "../../../types";

    let {
        rule,
        scenario,
        accountNames,
        commodity,
        styles,
        onEdit,
    }: {
        /** The draft's rule. Mutated in place. */
        rule: AutoRule;
        /** For the preview's sample amount. */
        scenario: Scenario;
        accountNames: string[];
        commodity: string;
        styles: ReadonlyMap<string, AmountStyle>;
        onEdit: () => void;
    } = $props();

    const locked = $derived(autoLockedReason(rule));
    const unscoped = $derived(rule.edited !== true && rule.raw !== "" && rule.unsupported === null && !rule.query.includes("generated-transaction"));

    /** From now on the form, not the text it was read as, is what a save sends. */
    function edited(): void {
        rule.edited = true;
        onEdit();
    }
</script>

<div class="flex flex-col gap-3" data-testid="auto-entry-editor">
    {#if rule.form !== null}
        <AutoRuleEditor
            form={rule.form}
            {accountNames}
            name="this rule"
            {commodity}
            sample={ruleSample(scenario, rule.form.matchAccount)}
            {styles}
            onChange={edited}
        />
        {#if unscoped}
            <div class="flex flex-wrap items-center gap-2 rounded-box bg-base-100/50 px-2 py-1.5 text-xs text-base-content/60">
                <span>Applied here to projected transactions only. Stock <code>hledger --auto</code> would also apply it to recorded ones.</span>
                <button type="button" class="btn btn-ghost btn-xs" onclick={edited}>Limit to forecasts in the file</button>
            </div>
        {/if}
    {:else}
        <div class="flex flex-col gap-1" data-testid="auto-locked">
            <p class="flex items-center gap-1.5 text-xs text-warning"><span aria-hidden="true">🔒</span>{locked}</p>
            <pre class="overflow-x-auto rounded-box bg-base-100/60 px-3 py-2 font-mono text-xs text-base-content/75">{rule.raw.trimEnd()}</pre>
            <p class="text-xs text-base-content/45">Edit this rule in the file. It can still be turned off, moved and commented here.</p>
        </div>
    {/if}
</div>
