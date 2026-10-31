<script lang="ts">
    // "Remember as a rule" (plan 32 D25): turn the change on an import card
    // into an `if` rule at the end of the import's rules file, so the next
    // statement arrives already categorised.
    //
    // Off by default (Q9): rules get written only when asked for. The overlay
    // shows this only on a CSV import's card whose description or account
    // changed (not a split), and binds `R` to the toggle.
    //
    //   Match     [description ▾] [ blue.*bottle.*oakland ]
    //   Then set  ☑ account2 → expenses:food:coffee
    //             ☐ description → Blue Bottle Coffee
    //   Matches 7 of 112 rows in checking.csv · 0 already categorised
    //
    // The MATCH is prefilled from the card's similarity key (D10), escaped and
    // joined with `.*`, scoped to the rules file's `description` field when the
    // dry run lined the CSV records up with its transactions, otherwise to the
    // whole record. THEN SET follows the card's edit live; each line is
    // checkable. The LIVE CHECK runs the import with the candidate appended to a
    // temporary copy of the rules file: debounced, each new one aborting the
    // last, and listing the rows it would change, so a pattern that is too broad
    // is plain before anything is saved. Nothing here writes: the overlay saves
    // the rule (through `source.save`) when the card is saved.
    import {untrack} from "svelte";
    import {classify} from "#lib/api/editFailure.js";
    import type {CardChange} from "#lib/api/editMapping.js";
    import type {Transaction} from "#lib/domain/types.js";
    import {
        changedLines,
        defaultMatchField,
        matchFields,
        RULE_PREVIEW_DEBOUNCE_MS,
        rulePattern,
        ruleSpec,
        ruleTargets,
        sourceText,
    } from "#lib/imports/ruleBuilder.js";
    import type {RulePreview, RuleSource, RuleSpec, StagedTxn} from "#lib/imports/reviewTypes.js";
    import ReviewIcon from "./ReviewIcon.svelte";

    /** Changed rows listed before "and N more". */
    const LISTED = 20;

    let {
        source,
        item,
        original,
        change,
        on = $bindable(false),
        error = null,
        busy = false,
    }: {
        source: RuleSource;
        /** The card: its ordinal finds its CSV record, its key the prefill. */
        item: StagedTxn;
        /** The card's transaction as generated, which `change` is measured from. */
        original: Transaction;
        change: CardChange;
        /** Remember as a rule. */
        on?: boolean;
        /** Why saving the rule failed, from the overlay. */
        error?: string | null;
        /** The rule is being saved. */
        busy?: boolean;
    } = $props();

    type Check = {kind: "idle"} | {kind: "unavailable"} | {kind: "done"; preview: RulePreview} | {kind: "error"; message: string};

    const ordinal = $derived(item.ref.kind === "csv" ? item.ref.ordinal : -1);
    const fields = $derived(matchFields(source.records));
    /** The prefill for `scope`: from its own CSV cell, or the description's for a whole-record match. */
    const prefill = (scope: string | null): string => rulePattern(item.key, sourceText(source.records, ordinal, scope ?? "description"));

    // Seeded once per card (the card remounts this): the user owns them from here.
    const initialField = untrack(() => defaultMatchField(source.records));
    let field = $state<string | null>(initialField);
    let pattern = $state(prefill(initialField));
    /** The pattern still reads as prefilled, so a new scope may re-prefill it. */
    let patternTouched = $state(false);
    /** The user's own ticks, by field; untouched lines take their default. */
    let ticked = $state<Record<string, boolean>>({});

    const targets = $derived(ruleTargets(original, change));
    const checkedFields = $derived(new Set(targets.filter((target) => ticked[target.field] ?? target.defaultOn).map((target) => target.field)));
    const spec = $derived(ruleSpec(field, pattern, targets, checkedFields));
    /** A string, so the live check reruns only when the rule itself changes, not on every edit elsewhere on the card. */
    const signature = $derived(spec === null ? null : JSON.stringify(spec));

    let check = $state<Check>({kind: "idle"});
    let checking = $state(false);
    let showChanges = $state(false);

    /** The rule as it stands, for the overlay to save. Null while it has no pattern or nothing to set. */
    export function currentRule(): RuleSpec | null {
        return spec;
    }

    // The live check. Debounced; a newer rule (or turning it off, or leaving
    // the card) aborts the request in flight and drops its answer.
    $effect(() => {
        const key = on ? signature : null;
        if (key === null) {
            checking = false;
            return;
        }
        const candidate = JSON.parse(key) as RuleSpec;
        const controller = new AbortController();
        checking = true;
        const timer = setTimeout(() => {
            void (async () => {
                try {
                    const preview = await source.preview(candidate, controller.signal);
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

    const ok = $derived(check.kind === "done" && check.preview.ok ? check.preview : null);
</script>

<section class="rounded-box border border-base-content/10 bg-base-200/40" data-testid="review-rule" data-on={on ? "" : undefined}>
    <label class="flex cursor-pointer items-center gap-3 px-3 py-2">
        <input type="checkbox" class="toggle toggle-sm toggle-success" bind:checked={on} disabled={busy} data-testid="review-rule-toggle" />
        <span class="flex min-w-0 grow flex-col">
            <span class="flex items-center gap-1.5 text-sm font-medium">
                <ReviewIcon name="bolt" class="h-3.5 w-3.5 text-base-content/50" />
                Remember as a rule
                <kbd class="kbd kbd-xs font-normal">⇧R</kbd>
            </span>
            <span class="truncate text-xs text-base-content/50">
                {on ? `Added to the end of ${source.rulesLabel} when you save this card` : `So the next import from ${source.fileLabel} does this by itself`}
            </span>
        </span>
    </label>

    <!-- 0fr → 1fr, like Similar: the builder opens without measuring anything. -->
    <div class={["grid transition-[grid-template-rows] duration-120 ease-out motion-reduce:transition-none", on ? "grid-rows-[1fr]" : "grid-rows-[0fr]"]}>
        <div class="min-h-0 overflow-hidden" inert={!on}>
            <div class="grid grid-cols-[4.5rem_minmax(0,1fr)] items-start gap-x-3 gap-y-3 border-t border-base-content/10 px-3 pt-3 pb-3">
                <span class="pt-1.5 text-[0.65rem] font-semibold tracking-[0.14em] text-base-content/45 uppercase">Match</span>
                <div class="flex min-w-0 flex-wrap items-center gap-2">
                    <select
                        class="select w-40 shrink-0 select-sm"
                        value={field ?? ""}
                        onchange={(event) => scopeTo(event.currentTarget.value)}
                        disabled={busy}
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
                        disabled={busy}
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
                            <label class="flex min-w-0 cursor-pointer items-center gap-2 text-sm">
                                <input
                                    type="checkbox"
                                    class="checkbox checkbox-xs checkbox-success"
                                    checked={checkedFields.has(target.field)}
                                    onchange={(event) => setTicked(target.field, event.currentTarget.checked)}
                                    disabled={busy}
                                    aria-label="Set {target.field}"
                                    data-testid="review-rule-target"
                                    data-field={target.field}
                                />
                                <span class="shrink-0 font-mono text-xs text-base-content/70">{target.field}</span>
                                <ReviewIcon name="arrow-right" class="h-3 w-3 shrink-0 text-base-content/35" />
                                <span
                                    class={[
                                        "truncate font-mono text-xs",
                                        checkedFields.has(target.field) ? "text-success" : "text-base-content/40 line-through",
                                    ]}>{target.value}</span
                                >
                            </label>
                        </li>
                    {/each}
                </ul>
            </div>

            <!-- The live check. -->
            <div class="border-t border-dashed border-base-content/15 px-3 py-2.5 text-xs" aria-live="polite" data-testid="review-rule-check">
                {#if spec === null}
                    <span class="text-base-content/50">Give the rule a pattern and something to set.</span>
                {:else if check.kind === "idle"}
                    <span class="flex items-center gap-2 text-base-content/50"><span class="loading loading-xs loading-spinner"></span> Checking the rule…</span
                    >
                {:else if check.kind === "unavailable"}
                    <span class="text-base-content/50">The live check needs a rules file and a journal chosen for this import.</span>
                {:else if check.kind === "error"}
                    <span class="text-error" role="alert" data-testid="review-rule-error">Couldn't check this rule: {check.message}</span>
                {:else if !check.preview.ok}
                    <div class="flex flex-col gap-1.5" role="alert" data-testid="review-rule-refused">
                        <span class="text-error">hledger can't use the rules file with this rule added:</span>
                        <pre class="max-h-32 overflow-auto rounded-field bg-base-300 p-2 text-[0.7rem] whitespace-pre-wrap">{check.preview.stderr}</pre>
                    </div>
                {:else if ok !== null}
                    <div class={["flex flex-wrap items-center gap-x-2 gap-y-1 transition-opacity", checking && "opacity-60"]}>
                        <span class={["h-1.5 w-1.5 shrink-0 rounded-full", ok.matched === 0 ? "bg-warning" : "bg-success"]}></span>
                        <span class="text-base-content/80" data-testid="review-rule-count">
                            Matches <strong class="text-base-content tabular-nums">{ok.matched}</strong> of
                            <span class="tabular-nums">{ok.total}</span>
                            rows in {source.fileLabel}
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
                    {#if ok.matched === 0}
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
        </div>
    </div>
</section>
