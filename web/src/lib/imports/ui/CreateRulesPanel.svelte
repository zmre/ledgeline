<script lang="ts">
    // "Nothing here can read this file" → "here is one that can."
    //
    // The one screen in the imports feature that WRITES A NEW FILE into the
    // user's journal directory, and it is built to look like a review rather
    // than a form: the engine has already read the CSV and made its guesses,
    // and everything on screen is that draft, shown so it can be corrected
    // before it is written.
    //
    // # Almost none of this is new, on purpose
    //
    // A drafted document is a `RulesDocument` like any other, so `toForm` makes
    // it the same `FormItem[]` the Edit Rules tab edits — which means the column
    // mapping is `RowMappingPanel` and the two accounts are `AccountsPanel`,
    // unchanged, editing through the same `withFieldNames`/`withSetting`. Two
    // consequences worth stating, because they are why this file is short:
    //
    //   - correcting a mis-detected column is the SAME control, with the same
    //     `%name` semantics and the same free-text escape hatch, as correcting
    //     one in an existing file. There is no second mapping UI to keep true.
    //   - the draft is fetched ONCE. Everything after it is a local edit of the
    //     returned document, and the save sends those typed items. Re-drafting
    //     on every change would throw away the corrections the user came here
    //     to make.
    //
    // Phase 1's `RuleSummaryCard`/`RulesList` are deliberately NOT reused: a
    // drafted file has no `if` rules in it at all (guessing a category from a
    // payee is a separate, out-of-scope piece of work), so a rules list would
    // render an empty state next to the only thing worth looking at. What the
    // user needs to see instead is the file itself, which is `draftLines`.
    //
    // No `AsyncSection`/`dataView` here, deliberately: those are for a surface
    // backed by a `createResource`, and this is a PRESSED action with a
    // one-line outcome — the same shape `installConfAliases` has. Its `view`
    // prop is documented as coming from `dataView` and never hand-rolled, so
    // synthesising one would be exactly the misuse that comment warns about.
    import {createBlocker, draftLines} from "../createModel";
    import type {RulesDraft} from "../types";
    import {settingText, withSetting, type FormItem, type RulesForm} from "../model";
    import AccountsPanel from "./AccountsPanel.svelte";
    import RowMappingPanel from "./RowMappingPanel.svelte";

    let {
        draft,
        form,
        id,
        drafting,
        saving,
        error,
        accountNames,
        onId,
        onItems,
        onSave,
        onRetry,
        onCancel,
    }: {
        /** The engine's draft, or null before the first one lands. */
        draft: RulesDraft | null;
        /** The editable working copy. Null exactly when `draft` is. */
        form: RulesForm | null;
        id: string;
        drafting: boolean;
        saving: boolean;
        error: string | null;
        accountNames: string[];
        onId: (value: string) => void;
        onItems: (items: FormItem[]) => void;
        onSave: () => void;
        onRetry: () => void;
        onCancel: () => void;
    } = $props();

    const blocker = $derived(form === null ? null : createBlocker(id, form));
    const lines = $derived(form === null ? [] : draftLines(form.items).filter((line) => line !== ""));

    /**
     * The confidence badge for a column, or null when there is nothing to say.
     *
     * Only the shaky ones are marked. A badge on every row is a badge nobody
     * reads, and the point of showing confidence at all is to draw the eye to
     * the guesses that need checking — a column read from its VALUES rather
     * than from its header is the one that is most often wrong.
     */
    function guessTone(index: number): {label: string; tone: string} | null {
        const guess = draft?.columns.find((column) => column.index === index);
        if (guess === undefined) return null;
        if (guess.field === null) return null;
        if (guess.confidence >= 0.9) return null;
        if (guess.confidence >= 0.6) return {label: "check this", tone: "badge-ghost"};
        return {label: "guess", tone: "badge-warning"};
    }

    const uncertain = $derived((draft?.columns ?? []).filter((column) => column.field !== null && column.confidence < 0.9).length);
</script>

<section class="flex flex-col gap-4 rounded-box border border-primary/30 bg-primary/5 p-3" data-testid="imports-create-rules-panel">
    <header class="flex flex-col gap-1">
        <h2 class="text-sm font-semibold tracking-tight">Create a rules file for this statement</h2>
        <p class="text-xs text-base-content/70">
            Ledgeline has read your file and guessed how its columns map onto hledger's fields. Check the mapping, say which account the statement is for, and
            it will write the rules file beside your journal. <strong>Nothing is written until you press Create.</strong>
        </p>
    </header>

    {#if drafting}
        <p class="flex items-center gap-2 text-sm text-base-content/70" role="status">
            <span class="loading loading-sm loading-spinner"></span>
            Reading your file…
        </p>
    {:else if form === null}
        <!-- No draft AND not loading: the request failed. The engine's own
             sentence, verbatim — it is the one that says whether the name was
             taken, the upload expired, or something else. -->
        <div class="flex flex-col items-start gap-2" data-testid="imports-create-error">
            <p class="text-sm text-error" role="alert">{error ?? "Ledgeline could not draft a rules file for this data."}</p>
            <button type="button" class="btn btn-sm" onclick={onRetry} data-testid="imports-create-retry">Try again</button>
        </div>
    {:else}
        <div class="flex flex-col gap-4">
            <!-- The name first: it decides WHERE the file goes, and a
                         rules file that is not named after its data file is one
                         hledger will not find. -->
            <div class="form-control">
                <label class="label-text text-xs" for="create-rules-id">File name</label>
                <input
                    id="create-rules-id"
                    type="text"
                    class="input w-full font-mono input-sm"
                    value={id}
                    disabled={saving}
                    autocomplete="off"
                    spellcheck="false"
                    data-testid="imports-create-id"
                    oninput={(event) => onId(event.currentTarget.value)}
                />
                <span class="label-text-alt mt-1 text-xs text-base-content/60">
                    Relative to your journal. hledger finds a rules file by name, so <code>bank.csv</code> is read through
                    <code>bank.csv.rules</code> beside it.
                </span>
            </div>

            <AccountsPanel items={form.items} {accountNames} disabled={saving} onChange={onItems} />

            <!-- The currency, and it is here because a WARNING points at it.
                 hledger reads a commodity-less amount as a commodity of its own,
                 so a statement of bare numbers never adds up with the `$`
                 amounts already in the journal — visible only as a balance that
                 does not move. The engine says so when it cannot infer one, and
                 a warning naming a control that did not exist would be worse
                 than no warning at all.

                 Left EMPTY when the amounts already carry a symbol: `currency`
                 is a blind string prefix, so declaring `$` over a cell reading
                 `$-4.50` produces `$$-4.50`, which is a different commodity and
                 is never reported. -->
            <div class="form-control max-w-xs">
                <label class="label-text text-xs" for="create-rules-currency">Currency</label>
                <input
                    id="create-rules-currency"
                    type="text"
                    class="input w-full font-mono input-sm"
                    value={settingText(form.items, "currency") ?? ""}
                    disabled={saving}
                    autocomplete="off"
                    spellcheck="false"
                    placeholder="$"
                    data-testid="imports-create-currency"
                    onchange={(event) => onItems(withSetting(form.items, "currency", event.currentTarget.value.trim()))}
                />
                <span class="label-text-alt mt-1 text-xs text-base-content/60">
                    Prepended to every amount. Leave it blank if the amounts in your file already carry a symbol — hledger would otherwise read
                    <code>$-4.50</code> as <code>$$-4.50</code>, a commodity of its own.
                </span>
            </div>

            <div class="flex flex-col gap-2">
                <div class="flex flex-wrap items-baseline justify-between gap-2">
                    <h3 class="text-sm font-semibold tracking-tight">Which column is which?</h3>
                    {#if uncertain > 0}
                        <span class="text-xs text-warning" data-testid="imports-create-uncertain">
                            {uncertain}
                            {uncertain === 1 ? "column was" : "columns were"} guessed — check {uncertain === 1 ? "it" : "them"} below.
                        </span>
                    {/if}
                </div>
                <!-- The same control the Edit Rules tab uses, over the
                             draft's own preview rows. `pending` is false: this
                             preview came WITH the draft rather than from a
                             second request, so it is never withheld. -->
                <RowMappingPanel items={form.items} preview={draft?.preview ?? null} pending={false} disabled={saving} onChange={onItems} />
                {#if draft !== null}
                    <div class="flex flex-wrap gap-1">
                        {#each draft.columns as column (column.index)}
                            {@const tone = guessTone(column.index)}
                            {#if tone !== null}
                                <span class="badge badge-sm {tone.tone}">Column {column.index + 1}: {tone.label}</span>
                            {/if}
                        {/each}
                    </div>
                {/if}
            </div>

            {#if draft !== null && draft.warnings.length > 0}
                <!-- Every one of these names a way the draft can be
                             wrong that hledger will NOT mention. -->
                <ul class="flex flex-col gap-1" data-testid="imports-create-warnings">
                    {#each draft.warnings as warning (warning)}
                        <li class="text-xs text-warning">{warning}</li>
                    {/each}
                </ul>
            {/if}

            <div class="flex flex-col gap-1">
                <h3 class="text-sm font-semibold tracking-tight">What the file will say</h3>
                <pre class="overflow-x-auto rounded bg-base-200 p-2 font-mono text-xs" data-testid="imports-create-lines">{lines.join("\n")}</pre>
                <p class="text-xs text-base-content/60">
                    Rows that no rule matches land in <code>expenses:unknown</code>, which is what makes them easy to find afterwards. Add rules for them in
                    <strong>Edit Rules</strong> once the file exists.
                </p>
            </div>

            {#if error !== null}
                <p class="text-sm text-error" role="alert" data-testid="imports-create-save-error">{error}</p>
            {/if}

            <div class="flex flex-wrap items-center gap-3">
                <button type="button" class="btn btn-primary btn-sm" disabled={blocker !== null || saving} onclick={onSave} data-testid="imports-create-save">
                    {#if saving}<span class="loading loading-xs loading-spinner"></span>{/if}
                    Create rules file
                </button>
                <button type="button" class="btn btn-ghost btn-sm" disabled={saving} onclick={onCancel} data-testid="imports-create-cancel"> Cancel </button>
                <!-- A disabled button with no explanation is how a form
                             dead-ends. Every blocker names a field above it. -->
                {#if blocker !== null}
                    <span class="text-sm text-warning" role="status" data-testid="imports-create-blocker">{blocker}</span>
                {/if}
            </div>
        </div>
    {/if}
</section>
