<script lang="ts">
    // The rule itself (plan 32 D25, D27): MATCH, THEN SET and the live check,
    // for ONE rules file. `RuleBuilder.svelte` is the toggle around it and
    // keys this on its source, so a journal card that switches rules file
    // starts the rule again from that file's fields (and its own check).
    //
    // The MATCH is prefilled by the source (an import from the card's raw CSV
    // cell, a journal card from its description), scoped to the field it
    // names. THEN SET follows the card's edit live; each line is checkable.
    // The LIVE CHECK runs the candidate against a temporary copy of the rules
    // file: debounced, each new one aborting the last, and listing the rows it
    // would change, so a pattern that is too broad is plain before anything
    // is saved. On a journal card with no statement to run it on, it counts the
    // journal's transactions from the file's account that the pattern meets.
    //
    // With no source (a journal card whose file the user has yet to pick) all
    // of it is disabled, with a line saying why.
    import {untrack} from "svelte";
    import {classify} from "#lib/api/editFailure.js";
    import type {CardChange} from "#lib/api/editMapping.js";
    import type {Transaction} from "#lib/domain/types.js";
    import {changedLines, fieldReferences, RULE_PREVIEW_DEBOUNCE_MS, ruleSpec, ruleTargets} from "#lib/imports/ruleBuilder.js";
    import type {RulePrefill, RulePreview, RuleSource, RuleSpec} from "#lib/imports/reviewTypes.js";
    import type {QueueItem} from "../types";
    import ReviewIcon from "./ReviewIcon.svelte";

    /** Changed rows listed before "and N more". */
    const LISTED = 20;
    /** No file picked: nothing to scope to, nothing prefilled. */
    const NO_PREFILL: RulePrefill = {fields: [], field: null, pattern: () => ""};

    let {
        source,
        item,
        original,
        change,
        on,
        error = null,
        busy = false,
    }: {
        /** The rules file the rule is for, or null while a journal card's is unpicked (everything disabled). */
        source: RuleSource | null;
        /** The card: the source builds the prefill from it. */
        item: QueueItem;
        /** The card's transaction as it was, which `change` is measured from. */
        original: Transaction;
        change: CardChange;
        /** Remember as a rule is on: the live check runs only then. */
        on: boolean;
        /** Why saving the rule failed, from the overlay. */
        error?: string | null;
        /** The rule is being saved. */
        busy?: boolean;
    } = $props();

    type Check = {kind: "idle"} | {kind: "unavailable"} | {kind: "done"; preview: RulePreview} | {kind: "error"; message: string};

    // Fixed per mount: the builder remounts this when the source changes.
    const rules = untrack(() => source);
    const starts = rules?.prefill ?? NO_PREFILL;
    const fields = starts.fields;
    const locked = $derived(busy || rules === null);
    /** The prefill for `scope`, as the source builds it. */
    const prefill = (scope: string | null): string => starts.pattern(item, scope);

    // Seeded once per mount: the user owns them from here.
    const initialField = starts.field;
    let field = $state<string | null>(initialField);
    let pattern = $state(untrack(() => prefill(initialField)));
    /** The pattern still reads as prefilled, so a new scope may re-prefill it. */
    let patternTouched = $state(false);
    /** The user's own ticks, by field; untouched lines take their default. */
    let ticked = $state<Record<string, boolean>>({});

    const targets = $derived(ruleTargets(original, change, rules?.account ?? null));
    const checkedFields = $derived(new Set(targets.filter((target) => ticked[target.field] ?? target.defaultOn).map((target) => target.field)));
    const spec = $derived(rules === null ? null : ruleSpec(field, pattern, targets, checkedFields));
    /** Checked values hledger would read `%name` in as a CSV field (it has no escape for a literal `%`). */
    const references = $derived(targets.filter((target) => checkedFields.has(target.field)).flatMap((target) => fieldReferences(target.value)));
    /** A string, so the live check reruns only when the rule itself changes, not on every edit elsewhere on the card. */
    const signature = $derived(spec === null ? null : JSON.stringify(spec));

    let check = $state<Check>({kind: "idle"});
    let checking = $state(false);
    let showChanges = $state(false);

    /** The rule as it stands, for the overlay to save. Null while it has no file, no pattern or nothing to set. */
    export function currentRule(): RuleSpec | null {
        return spec;
    }

    // The live check. Debounced; a newer rule (or turning it off, or leaving
    // the card) aborts the request in flight and drops its answer.
    $effect(() => {
        const key = on ? signature : null;
        if (key === null || rules === null) {
            checking = false;
            return;
        }
        const candidate = JSON.parse(key) as RuleSpec;
        const controller = new AbortController();
        checking = true;
        const timer = setTimeout(() => {
            void (async () => {
                try {
                    const preview = await rules.preview(candidate, controller.signal);
                    if (controller.signal.aborted) return;
                    check = preview === null ? {kind: "unavailable"} : {kind: "done", preview};
                } catch (cause) {
                    if (controller.signal.aborted) return;
                    check = {kind: "error", message: classify(cause).message};
                } finally {
                    if (!controller.signal.aborted) checking = false;
                }
            })();
        }, RULE_PREVIEW_DEBOUNCE_MS);
        return () => {
            clearTimeout(timer);
            controller.abort();
        };
    });

    function scopeTo(value: string): void {
        const next = value === "" ? null : value;
        if (!patternTouched) pattern = prefill(next);
        field = next;
    }

    function setTicked(target: string, value: boolean): void {
        ticked = {...ticked, [target]: value};
    }

    const answer = $derived(check.kind === "done" && check.preview.ok ? check.preview : null);
    /** A journal card's fallback count (D27): no statement to run the rule on. */
    const counted = $derived(answer !== null && answer.basis === "journal" ? answer : null);
    /** A run over CSV rows: the import's, or a journal card's against the file's newest statement. */
    const ok = $derived(answer !== null && answer.basis !== "journal" ? answer : null);
</script>

<div
    class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-start gap-x-3 gap-y-3 px-3 pt-3 pb-3"
    data-testid="review-rule-body"
    data-locked={rules === null ? "" : undefined}
>
    <span class="pt-1.5 text-[0.65rem] font-semibold tracking-[0.14em] text-base-content/45 uppercase">Match</span>
    <div class="flex min-w-0 flex-wrap items-center gap-2">
        <select
            class="select w-40 shrink-0 select-sm"
            value={field ?? ""}
            onchange={(event) => scopeTo(event.currentTarget.value)}
            disabled={locked}
            aria-label="Match against"
            data-testid="review-rule-field"
        >
            <option value="">whole record</option>
            {#each fields as name (name)}
                <option value={name}>{name}</option>
            {/each}
        </select>
        <input
            class="input min-w-0 grow font-mono input-sm"
            bind:value={pattern}
            oninput={() => (patternTouched = true)}
            disabled={locked}
            spellcheck="false"
            autocomplete="off"
            aria-label="Pattern"
            placeholder="a regular expression"
            data-testid="review-rule-pattern"
        />
        <span
            class="tooltip tooltip-left shrink-0"
            data-tip="A regular expression, matched case-insensitively anywhere in the field. .* stands for anything in between."
        >
            <span class="grid h-5 w-5 place-items-center rounded-full border border-base-content/20 text-[0.65rem] text-base-content/50">i</span>
        </span>
    </div>

    <span class="pt-0.5 text-[0.65rem] font-semibold tracking-[0.14em] text-base-content/45 uppercase">Then set</span>
    <ul class="flex min-w-0 flex-col gap-1.5" data-testid="review-rule-targets">
        {#each targets as target (target.field)}
            <li>
                <label class={["flex min-w-0 items-center gap-2 text-sm", !locked && "cursor-pointer"]}>
                    <input
                        type="checkbox"
                        class="checkbox checkbox-xs checkbox-success"
                        checked={checkedFields.has(target.field)}
                        onchange={(event) => setTicked(target.field, event.currentTarget.checked)}
                        disabled={locked}
                        aria-label="Set {target.field}"
                        data-testid="review-rule-target"
                        data-field={target.field}
                    />
                    <span class="shrink-0 font-mono text-xs text-base-content/70">{target.field}</span>
                    <ReviewIcon name="arrow-right" class="h-3 w-3 shrink-0 text-base-content/35" />
                    <span class={["truncate font-mono text-xs", checkedFields.has(target.field) ? "text-success" : "text-base-content/40 line-through"]}
                        >{target.value}</span
                    >
                </label>
            </li>
        {/each}
    </ul>
    {#if references.length > 0}
        <p class="col-start-2 text-xs text-warning" role="alert" data-testid="review-rule-percent">
            hledger reads <code>{references.join(" ")}</code> as a CSV field and puts that field's value, or nothing, in its place. It has no way to write a literal
            %. Reword the change, or untick that line.
        </p>
    {/if}
</div>

<!-- The live check. -->
<div class="border-t border-dashed border-base-content/15 px-3 py-2.5 text-xs" aria-live="polite" data-testid="review-rule-check">
    {#if rules === null}
        <span class="text-base-content/50" data-testid="review-rule-unpicked">Choose a rules file first: what a rule can match depends on the file.</span>
    {:else if spec === null}
        <span class="text-base-content/50">Give the rule a pattern and something to set.</span>
    {:else if check.kind === "idle"}
        <span class="flex items-center gap-2 text-base-content/50"><span class="loading loading-xs loading-spinner"></span> Checking the rule…</span>
    {:else if check.kind === "unavailable"}
        <span class="text-base-content/50">The live check needs a rules file and a journal chosen for this import.</span>
    {:else if check.kind === "error"}
        <span class="text-error" role="alert" data-testid="review-rule-error">Couldn't check this rule: {check.message}</span>
    {:else if !check.preview.ok}
        <div class="flex flex-col gap-1.5" role="alert" data-testid="review-rule-refused">
            <span class="text-error">hledger can't use the rules file with this rule added:</span>
            <pre class="max-h-32 overflow-auto rounded-field bg-base-300 p-2 text-[0.7rem] whitespace-pre-wrap">{check.preview.stderr}</pre>
        </div>
    {:else if counted !== null}
        <div class={["flex flex-wrap items-center gap-x-2 gap-y-1 transition-opacity", checking && "opacity-60"]}>
            <span class={["h-1.5 w-1.5 shrink-0 rounded-full", counted.matched === 0 ? "bg-warning" : "bg-success"]}></span>
            <span class="text-base-content/80" data-testid="review-rule-count" data-basis="journal">
                Matches <strong class="text-base-content tabular-nums">{counted.matched}</strong> of
                <span class="tabular-nums">{counted.total}</span>
                transactions from <span class="font-mono">{rules.account ?? "this account"}</span>
                <span class="text-base-content/50">(no statement file found to test against)</span>
            </span>
            {#if checking}<span class="loading loading-xs loading-spinner text-base-content/50"></span>{/if}
        </div>
        {#if counted.matched === 0}
            <p class="mt-1 text-warning">No transaction from it matches, this one included. Loosen the pattern.</p>
        {/if}
    {:else if ok !== null}
        <div class={["flex flex-wrap items-center gap-x-2 gap-y-1 transition-opacity", checking && "opacity-60"]}>
            <span class={["h-1.5 w-1.5 shrink-0 rounded-full", ok.matched === 0 ? "bg-warning" : "bg-success"]}></span>
            <span class="text-base-content/80" data-testid="review-rule-count" data-basis={ok.basis ?? "import"}>
                Matches <strong class="text-base-content tabular-nums">{ok.matched}</strong> of
                <span class="tabular-nums">{ok.total}</span>
                rows in {ok.file ?? rules.fileLabel}
                <span class="text-base-content/40">·</span>
                <span class="tabular-nums">{ok.alreadyCategorised}</span> already categorised
            </span>
            {#if checking}<span class="loading loading-xs loading-spinner text-base-content/50"></span>{/if}
            <span class="grow"></span>
            {#if ok.changedTotal > 0}
                <button
                    type="button"
                    class="flex items-center gap-1 text-base-content/60 hover:text-base-content"
                    aria-expanded={showChanges}
                    onclick={() => (showChanges = !showChanges)}
                    data-testid="review-rule-changes-toggle"
                >
                    <ReviewIcon name={showChanges ? "chevron-down" : "chevron-right"} class="h-3 w-3" />
                    {ok.changedTotal} would change
                </button>
            {/if}
        </div>
        {#if ok.noCsvMatch === true}
            <p class="mt-1 text-warning" data-testid="review-rule-renamed">
                No rows in {ok.file ?? rules.fileLabel} match — this description may have been renamed by another rule; try matching the bank's original text.
            </p>
        {:else if ok.matched === 0}
            <p class="mt-1 text-warning">No row in this import matches — this card included. Loosen the pattern.</p>
        {/if}
        {#if showChanges && ok.changed.length > 0}
            <ul class="mt-2 flex max-h-48 flex-col gap-1.5 overflow-y-auto" data-testid="review-rule-changes">
                {#each ok.changed.slice(0, LISTED) as row (row.ordinal)}
                    {@const lines = changedLines(row.before, row.after)}
                    <li class="rounded-field bg-base-100/60 px-2 py-1 font-mono text-[0.7rem] leading-snug" data-testid="review-rule-change">
                        <span class="block truncate text-base-content/80">{lines.title}</span>
                        {#each lines.removed as line, at (`-${at}`)}
                            <span class="block truncate text-error/80">− {line}</span>
                        {/each}
                        {#each lines.added as line, at (`+${at}`)}
                            <span class="block truncate text-success">+ {line}</span>
                        {/each}
                    </li>
                {/each}
            </ul>
            {#if ok.changedTotal > Math.min(LISTED, ok.changed.length)}
                <p class="mt-1 text-base-content/50">…and {ok.changedTotal - Math.min(LISTED, ok.changed.length)} more</p>
            {/if}
        {/if}
    {/if}
    {#if error !== null}
        <p class="mt-1.5 text-error" role="alert" data-testid="review-rule-save-error">The rule wasn't saved: {error}</p>
    {/if}
</div>
