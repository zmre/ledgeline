<!-- Settings › Automatic postings (plan 30 Phase 5): every `= QUERY` rule in
     the main journal and the files it includes, with where it lives, what it
     says, whether this engine can read it, how many postings it generated, and
     anything hledger would object to. Plus whether `--auto` is in force at all
     — which is the hledger.conf's to say, so it is shown here and changed on
     the hledger config tab, never a second switch.

     A generated posting anywhere in the app links here with `?rule=N`, and
     that rule is scrolled to and outlined.

     Read-only for now. EDITOR SEAM: rule editing arrives with the shared rule
     editor component (branch `2026-10-28-rule-editor`). Pass it as `editor`;
     it is rendered inside each rule's card with that rule, and nothing else
     here has to change. -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import AsyncSection from "#lib/components/AsyncSection.svelte";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {autoRules, ensureAutoRules, reloadAutoRules} from "../store.svelte";
    import type {AutoRule} from "../types";

    let {focus, editor}: {focus?: number; editor?: Snippet<[AutoRule]>} = $props();

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
                        turn it on in the hledger config settings, or in that file.
                    </span>
                </div>
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
                                </div>
                                {#if rule.unsupported !== null}
                                    <p class="text-xs text-base-content/60">
                                        This engine cannot read this rule's query ({rule.unsupported}), so it never fires here.
                                    </p>
                                {/if}
                                <pre class="overflow-x-auto rounded bg-base-200 p-2 font-mono text-xs">{rule.raw.trimEnd()}</pre>
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
                                {#if editor !== undefined}
                                    {@render editor(rule)}
                                {/if}
                            </div>
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    {/snippet}
</AsyncSection>
