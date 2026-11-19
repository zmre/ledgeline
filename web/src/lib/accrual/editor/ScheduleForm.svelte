<script lang="ts">
    // The "Recognize over time" form (D-editor): customer/vendor, service
    // start, term (chips or an end date), the account to recognize into,
    // timing, and contract. Every change goes out through `onChange` at once;
    // the section writes the source tags onto the card and re-asks the engine
    // for the preview. Optional dates use DateInput (WebKit paints today into
    // an empty native date field).
    import DateInput from "#lib/components/DateInput.svelte";
    import {CONTRACT_KINDS, type ContractKind, type Party} from "../types";
    import {defaultDescription, TERM_CHIPS, type ScheduleDraft} from "../model";

    let {
        draft,
        label,
        partyKind,
        sourceDescription,
        accountNames,
        partyNames,
        inferredContract,
        partySuggestion = null,
        banner = null,
        endOnly = false,
        disabled = false,
        onChange,
    }: {
        draft: ScheduleDraft;
        /** "Schedule A". */
        label: string;
        /** Revenue sides have customers, expense sides vendors. */
        partyKind: Party["kind"];
        /** The source transaction's description: the entry description's fallback. */
        sourceDescription: string;
        accountNames: readonly string[];
        /** Known customer (or vendor) names, for the field's completion. */
        partyNames: readonly string[];
        /** The contract the engine infers ("renewal" with an earlier schedule), marked in the select. */
        inferredContract: ContractKind;
        /** A party guessed from the payee: offered as a chip, never applied silently. */
        partySuggestion?: Party | null;
        /** "Prefilled from Customer X · Nov 2025 · 12 months · k3d8w1". */
        banner?: string | null;
        /** End contract: only the end date is editable. */
        endOnly?: boolean;
        disabled?: boolean;
        onChange: (next: ScheduleDraft) => void;
    } = $props();

    const uid = $props.id();
    const usesEndDate = $derived(draft.term.through !== undefined);

    function patch(changes: Partial<ScheduleDraft>): void {
        onChange({...draft, ...changes});
    }

    function setParty(name: string): void {
        const party = name.trim() === "" ? null : {kind: partyKind, name};
        // The description follows the party until the user writes their own.
        const auto = draft.description === defaultDescription(draft.party, sourceDescription);
        patch({party, ...(auto ? {description: defaultDescription(party, sourceDescription)} : {})});
    }

    function setTarget(account: string): void {
        patch({lines: draft.lines.map((line) => ({...line, targetAccount: account}))});
    }
</script>

<fieldset class="flex flex-col gap-3 text-sm" {disabled} data-testid="accrual-schedule-form">
    <legend class="mb-1 text-xs font-semibold tracking-wide text-base-content/60 uppercase">{label} · {draft.id}</legend>
    {#if banner !== null}
        <p class="rounded bg-info/10 px-2 py-1 text-xs" data-testid="accrual-renewal-banner">{banner}</p>
    {/if}
    {#if !endOnly}
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">{partyKind === "vendor" ? "Vendor" : "Customer"} (optional)</span>
            <input
                class="input input-sm"
                list="{uid}-parties"
                value={draft.party?.name ?? ""}
                oninput={(event) => setParty(event.currentTarget.value)}
                data-testid="accrual-party"
            />
            <datalist id="{uid}-parties">
                {#each partyNames as name (name)}<option value={name}></option>{/each}
            </datalist>
        </label>
        {#if partySuggestion !== null && draft.party === null}
            <button
                type="button"
                class="btn self-start btn-ghost btn-xs"
                onclick={() => partySuggestion !== null && setParty(partySuggestion.name)}
                data-testid="accrual-party-suggestion"
            >
                {partySuggestion.name}?
            </button>
        {/if}
        <div class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Service start</span>
            <DateInput
                value={draft.start}
                aria-label="Service start"
                onchange={(event) => patch({start: event.currentTarget.value})}
                data-testid="accrual-start"
            />
        </div>
        <div class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Term</span>
            <div class="flex flex-wrap items-center gap-1" role="group" aria-label="Term">
                {#each TERM_CHIPS as months (months)}
                    <button
                        type="button"
                        class={["btn btn-xs", draft.term.months === months ? "btn-primary" : "btn-ghost"]}
                        aria-pressed={draft.term.months === months}
                        onclick={() => patch({term: {months}})}
                        data-testid="accrual-term-{months}">{months} mo</button
                    >
                {/each}
                <button
                    type="button"
                    class={["btn btn-xs", usesEndDate ? "btn-primary" : "btn-ghost"]}
                    aria-pressed={usesEndDate}
                    onclick={() => patch({term: {through: draft.term.through ?? ""}})}
                    data-testid="accrual-term-end">End date</button
                >
            </div>
        </div>
    {/if}
    {#if usesEndDate || endOnly}
        <div class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">{endOnly ? "End contract on" : "Service end"}</span>
            <DateInput
                value={draft.term.through ?? ""}
                aria-label={endOnly ? "End contract on" : "Service end"}
                onchange={(event) => patch({term: {through: event.currentTarget.value}})}
                data-testid="accrual-end"
            />
        </div>
    {/if}
    {#if !endOnly}
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Recognize into</span>
            <input
                class="input input-sm"
                list="{uid}-accounts"
                value={draft.lines[0]?.targetAccount ?? ""}
                oninput={(event) => setTarget(event.currentTarget.value)}
                data-testid="accrual-target"
            />
            <datalist id="{uid}-accounts">
                {#each accountNames as name (name)}<option value={name}></option>{/each}
            </datalist>
        </label>
        <div class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Timing</span>
            <label class="flex items-center gap-2">
                <input
                    type="radio"
                    class="radio radio-xs"
                    name="{uid}-timing"
                    checked={draft.proration === "anniversary"}
                    onchange={() => patch({proration: "anniversary"})}
                />
                On the start date's anniversary
            </label>
            <label class="flex items-center gap-2">
                <input
                    type="radio"
                    class="radio radio-xs"
                    name="{uid}-timing"
                    checked={draft.proration === "calendar"}
                    onchange={() => patch({proration: "calendar"})}
                    data-testid="accrual-calendar"
                />
                Calendar months (anchored, prorated)
            </label>
        </div>
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Contract</span>
            <select
                class="select select-sm"
                value={draft.contract ?? ""}
                onchange={(event) => patch({contract: event.currentTarget.value === "" ? null : (event.currentTarget.value as ContractKind)})}
                data-testid="accrual-contract"
            >
                <option value="">—</option>
                {#each CONTRACT_KINDS as kind (kind)}
                    <option value={kind}>{kind}{kind === inferredContract ? " (inferred)" : ""}</option>
                {/each}
            </select>
        </label>
        <label class="flex flex-col gap-1">
            <span class="text-xs text-base-content/60">Entry description</span>
            <input
                class="input input-sm"
                value={draft.description}
                oninput={(event) => patch({description: event.currentTarget.value})}
                data-testid="accrual-description"
            />
        </label>
    {/if}
</fieldset>
