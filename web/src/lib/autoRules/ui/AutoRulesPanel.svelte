<!-- Settings › Automatic postings (plan 30 Phase 5): every `= QUERY` rule in
     the main journal and the files it includes, with where it lives, what it
     says, whether this engine can read it, how many postings it generated, and
     anything hledger would object to. Plus whether `--auto` is in force at all
     — which is the hledger.conf's to say, so it is shown here and changed on
     the hledger config tab, never a second switch.

     A generated posting anywhere in the app links here with `?rule=N`, and
     that rule is scrolled to and outlined.

     Editable (plan 30 amendment 22) when the server writes and the rule's file
     can be written: Edit opens the shared rule editor on the rule's form, or
     a text box on its block when a form cannot state it (or the user asks
     for the text); Delete asks once more; "+ Add a rule" appends to the main
     journal. Each is one `PUT /api/auto-rules`, which rewrites that block
     alone and answers 409 if the file moved — the listing is then re-read
     and the user re-applies. Nothing is saved until Save: a rule half-typed
     into a journal would change every report the moment it parsed. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import AutoRuleEditor from "../AutoRuleEditor.svelte";
    import {blankRuleForm, cloneRuleForm, ruleFormComplete, ruleFormToWire, type AutoRuleForm} from "../ruleForm";
    import {autoRules, ensureAutoRules, reloadAutoRules, saveAutoRule} from "../store.svelte";
    import type {AutoRule, AutoRuleIn, AutoRulesListing, SaveAutoRuleBody} from "../types";

    let {focus}: {focus?: number} = $props();

    /** What is being edited: a rule's form, or its block as text. */
    type Draft = {mode: "form"; form: AutoRuleForm} | {mode: "text"; raw: string};

    /** The one open editor: an existing rule (by `index`) or a new one. */
    let editing = $state<{key: number | "new"; draft: Draft; error: string | null} | null>(null);
    /** The rule whose Delete is waiting for its confirmation. */
    let confirmingDelete = $state<number | null>(null);
    let saving = $state(false);

    /** A new block's text, for a user who would rather type it. */
    const NEW_RULE_TEXT = "= \n    ";

    // Keyed on the generation, so an edit or an `--auto` toggle re-reads the
    // counts the moment the journal changes.
    $effect(() => {
        const url = settings.serverUrl;
        const generation = journal.generation;
        if (url !== null) ensureAutoRules(url, generation);
    });

    const listing = $derived(autoRules.value);

    function retry(): void {
        const url = settings.serverUrl;
        if (url !== null) reloadAutoRules(url, journal.generation);
    }

    /** Bring the focused rule into view once it has rendered. */
    function focused(node: HTMLElement, isFocused: boolean): void {
        if (isFocused) node.scrollIntoView?.({block: "center"});
    }

    function canEdit(listing: AutoRulesListing, rule: AutoRule): boolean {
        return listing.editable && rule.journalId !== null && rule.revision !== null;
    }

    function startEdit(rule: AutoRule): void {
        confirmingDelete = null;
        editing = {key: rule.index, draft: rule.form === null ? {mode: "text", raw: rule.raw} : {mode: "form", form: cloneRuleForm(rule.form)}, error: null};
    }

    function startAdd(): void {
        confirmingDelete = null;
        // A main-journal rule applies to recorded transactions, so not forecast-only.
        editing = {key: "new", draft: {mode: "form", form: blankRuleForm(null, false)}, error: null};
    }

    /** Switch an open editor to the text box: the rule as last saved, or a new block's skeleton. */
    function asText(rule: AutoRule | null): void {
        if (editing === null) return;
        editing.draft = {mode: "text", raw: rule?.raw ?? NEW_RULE_TEXT};
        editing.error = null;
    }

    /** The draft as `PUT` takes it, or the sentence saying why it cannot be sent yet. */
    function ruleIn(draft: Draft): AutoRuleIn | string {
        if (draft.mode === "text") return draft.raw.trim() === "" ? "Type the rule, starting with its = line." : {raw: draft.raw};
        if (!ruleFormComplete(draft.form)) return "Choose the account to match, and an account and a number for every posting.";
        return {form: ruleFormToWire(draft.form)};
    }

    async function send(body: SaveAutoRuleBody): Promise<string | null> {
        const url = settings.serverUrl;
        if (url === null) return "No server is configured.";
        saving = true;
        try {
            const outcome = await saveAutoRule(url, body);
            if (outcome.ok) return null;
            if (outcome.failure.kind === "conflict")
                return "The file changed since this list was read, so nothing was written. It has been re-read: make the change again.";
            return outcome.failure.message;
        } finally {
            saving = false;
        }
    }

    async function save(listing: AutoRulesListing, rule: AutoRule | null): Promise<void> {
        if (editing === null) return;
        const sent = ruleIn(editing.draft);
        if (typeof sent === "string") {
            editing.error = sent;
            return;
        }
        let body: SaveAutoRuleBody;
        if (rule === null) {
            if (listing.newRule === null) return;
            body = {...listing.newRule, change: {kind: "add", rule: sent}};
        } else {
            if (rule.journalId === null || rule.revision === null) return;
            body = {journalId: rule.journalId, revision: rule.revision, change: {kind: "replace", block: rule.block, was: rule.raw, rule: sent}};
        }
        const error = await send(body);
        if (editing === null) return;
        if (error === null) editing = null;
        else editing.error = error;
    }

    async function remove(rule: AutoRule): Promise<void> {
        if (rule.journalId === null || rule.revision === null) return;
        const error = await send({journalId: rule.journalId, revision: rule.revision, change: {kind: "remove", block: rule.block, was: rule.raw}});
        confirmingDelete = null;
        if (error !== null) editing = {key: rule.index, draft: {mode: "text", raw: rule.raw}, error};
    }

    function location(rule: AutoRule): string {
        return `${rule.file}:${rule.line}`;
    }

    /** A rule's problems with repeats folded: the same sentence for forty transactions is one line. */
    function grouped(rule: AutoRule): {message: string; kind: AutoRule["problems"][number]["kind"]; times: number}[] {
        const out: {message: string; kind: AutoRule["problems"][number]["kind"]; times: number}[] = [];
        for (const problem of rule.problems) {
            const seen = out.find((entry) => entry.message === problem.message);
            if (seen === undefined) out.push({message: problem.message, kind: problem.kind, times: 1});
            else seen.times += 1;
        }
        return out;
    }
</script>

{#snippet ruleEditor(listing: AutoRulesListing, rule: AutoRule | null)}
    {#if editing !== null}
        {@const name = rule === null ? "the new rule" : `rule ${rule.index + 1}`}
        <div class="flex flex-col gap-2 rounded-box bg-base-200/50 p-3" data-testid="settings-auto-editor">
            {#if editing.draft.mode === "form"}
                <AutoRuleEditor
                    form={editing.draft.form}
                    accountNames={journal.accountNames}
                    {name}
                    commodity={journal.dominantCommodity}
                    styles={journal.reportStyles}
                    scope="journal"
                    onChange={() => {
                        if (editing !== null) editing.error = null;
                    }}
                />
            {:else}
                <textarea
                    class="textarea w-full font-mono text-xs"
                    rows={Math.max(4, editing.draft.raw.split("\n").length + 1)}
                    bind:value={editing.draft.raw}
                    aria-label="Text of {name}"
                    spellcheck="false"
                    data-testid="settings-auto-text"></textarea>
                <p class="text-xs text-base-content/60">
                    One <code>= QUERY</code> line, then its postings, each indented. Written to the file exactly as typed.
                </p>
            {/if}
            {#if editing.error !== null}
                <div class="alert py-1.5 text-xs alert-error" role="alert" data-testid="settings-auto-save-error"><span>{editing.error}</span></div>
            {/if}
            <div class="flex flex-wrap items-center gap-2">
                <button type="button" class="btn btn-primary btn-xs" disabled={saving} onclick={() => void save(listing, rule)} data-testid="settings-auto-save"
                    >{saving ? "Saving…" : rule === null ? "Add rule" : "Save"}</button
                >
                <button type="button" class="btn btn-ghost btn-xs" disabled={saving} onclick={() => (editing = null)}>Cancel</button>
                {#if editing.draft.mode === "form"}
                    <span class="grow"></span>
                    <button
                        type="button"
                        class="btn btn-ghost btn-xs"
                        title="Edit the rule's text instead. Changes made in the form are dropped."
                        onclick={() => asText(rule)}>Edit as text</button
                    >
                {/if}
            </div>
        </div>
    {/if}
{/snippet}

<AsyncSection
    view={autoRules.view}
    value={listing}
    error={autoRules.error}
    testid="settings-auto-error"
    label="the automatic posting rules"
    loadingLabel="Reading the automatic posting rules"
    onRetry={retry}
>
    {#snippet children(loaded)}
        <div class="flex max-w-4xl flex-col gap-4" data-testid="settings-auto">
            <p class="text-sm text-base-content/70">
                An automatic posting rule (<code>= QUERY</code> followed by postings) adds those postings to every transaction the query matches — hledger's
                <em>auto postings</em>. Ledgeline shows what they add but never writes it into a transaction: to change a generated posting, change its rule.
            </p>

            {#if loaded.confHijacked}
                <div class="alert py-2 text-sm alert-warning" role="alert" data-testid="settings-auto-hijacked">
                    <span>
                        <code>{loaded.conf}</code> replaces hledger's command word, so none of its options reach any command —
                        <code>--auto</code> included. Rules are not applied.
                    </span>
                </div>
            {:else if loaded.inForce}
                <div class="alert py-2 text-sm alert-success" role="status" data-testid="settings-auto-in-force">
                    <span>Applied: <code>--auto</code> is set in <code>{loaded.conf}</code>, so these rules add postings to your journal.</span>
                </div>
            {:else}
                <div class="alert py-2 text-sm alert-info" role="status" data-testid="settings-auto-off">
                    <span>
                        Not applied: {#if loaded.conf === null}no <code>hledger.conf</code> is in force{:else}<code>{loaded.conf}</code> has no
                            <code>--auto</code> in its general section{/if}. hledger applies these rules only under <code>--auto</code>, and so does Ledgeline —
                        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/settings") IS the route id; the query string is appended -->
                        turn it on in the <a class="link" href="{resolve('/settings')}?tab=hledgerConf" data-testid="settings-auto-conf-link">hledger config</a> settings,
                        or in that file.
                    </span>
                </div>
            {/if}

            {#if loaded.editable && loaded.newRule !== null}
                {#if editing?.key === "new"}
                    {@render ruleEditor(loaded, null)}
                {:else}
                    <div>
                        <button type="button" class="btn btn-ghost btn-sm" onclick={startAdd} data-testid="settings-auto-add"
                            >+ Add a rule to <code>{loaded.newRule.journalId}</code></button
                        >
                    </div>
                {/if}
            {/if}

            {#if loaded.rules.length === 0}
                <p class="text-sm text-base-content/60" data-testid="settings-auto-empty">This journal has no automatic posting rules.</p>
            {:else}
                <ul class="flex flex-col gap-3">
                    {#each loaded.rules as rule (rule.index)}
                        {@const isFocused = rule.index === focus}
                        <li
                            class="card border border-base-content/10 bg-base-100 {isFocused ? 'ring-2 ring-primary' : ''}"
                            data-testid="settings-auto-rule"
                            data-rule={rule.index}
                            aria-current={isFocused ? "true" : undefined}
                            use:focused={isFocused}
                        >
                            <div class="card-body gap-2 p-4">
                                <div class="flex flex-wrap items-center gap-2">
                                    <code class="font-semibold">= {rule.query}</code>
                                    <span class="font-mono text-xs text-base-content/50">{location(rule)}</span>
                                    {#if rule.unsupported !== null}
                                        <span class="badge badge-sm badge-warning" title={rule.unsupported} data-testid="settings-auto-locked"
                                            >🔒 not applied</span
                                        >
                                    {:else if loaded.inForce}
                                        <span class="badge badge-ghost badge-sm"
                                            >{rule.generated} generated {rule.generated === 1 ? "posting" : "postings"}</span
                                        >
                                    {/if}
                                    {#if canEdit(loaded, rule) && editing?.key !== rule.index}
                                        <span class="grow"></span>
                                        {#if confirmingDelete === rule.index}
                                            <span class="text-xs text-base-content/70">Delete this rule from <code>{rule.file}</code>?</span>
                                            <button
                                                type="button"
                                                class="btn btn-error btn-xs"
                                                disabled={saving}
                                                onclick={() => void remove(rule)}
                                                data-testid="settings-auto-confirm-delete">Delete</button
                                            >
                                            <button type="button" class="btn btn-ghost btn-xs" onclick={() => (confirmingDelete = null)}>Keep</button>
                                        {:else}
                                            <button type="button" class="btn btn-ghost btn-xs" onclick={() => startEdit(rule)} data-testid="settings-auto-edit"
                                                >Edit</button
                                            >
                                            <button
                                                type="button"
                                                class="btn btn-ghost text-error btn-xs"
                                                onclick={() => (confirmingDelete = rule.index)}
                                                data-testid="settings-auto-delete">Delete</button
                                            >
                                        {/if}
                                    {/if}
                                </div>
                                {#if rule.unsupported !== null}
                                    <p class="text-xs text-base-content/60">
                                        This engine cannot read this rule's query ({rule.unsupported}), so it never fires here.
                                    </p>
                                {/if}
                                {#if editing?.key === rule.index}
                                    {@render ruleEditor(loaded, rule)}
                                {:else}
                                    <pre class="overflow-x-auto rounded bg-base-200 p-2 font-mono text-xs">{rule.raw.trimEnd()}</pre>
                                {/if}
                                {#each grouped(rule) as problem (problem.message)}
                                    <div
                                        class="alert py-1.5 text-xs {problem.kind === 'unknownAccount' ? 'alert-warning' : 'alert-error'}"
                                        role="alert"
                                        data-testid="settings-auto-problem"
                                    >
                                        <span
                                            >{problem.message}{#if problem.times > 1}&nbsp;(in {problem.times} transactions){/if}</span
                                        >
                                    </div>
                                {/each}
                            </div>
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    {/snippet}
</AsyncSection>
