<script lang="ts">
    // What your aliases do to this import — ONE panel, not two.
    //
    // # Why one
    //
    // This was two alerts stacked on the dry run, and they were reported as
    // "two alert boxes … both saying much the same thing about aliases". They do
    // not say the same thing:
    //
    //   - "your journal's aliases rewrite these account names in this import" is
    //     what WILL happen, to the entries shown directly below it;
    //   - "run from the command line, this same import would file them
    //     differently" is where it will NOT happen.
    //
    // But they were presented identically — an alias sentence, then a bulleted
    // list of `from → to` pairs — and the distinction lives in the second half
    // of each sentence, which is exactly the part a reader skimming two similar
    // boxes does not reach. So the difference was real and invisible, which on
    // screen is the same as not being there.
    //
    // One panel makes the relationship explicit instead of leaving it to be
    // inferred: the rewrite is the primary statement, and the parity divergence
    // is a caveat ABOUT that rewrite, subordinated under a rule and set smaller.
    // It is also true — the accounts a terminal would file differently are the
    // ones these aliases just rewrote, so the second list is a commentary on the
    // first, and it read as a repetition mostly because it was printed as a
    // sibling.
    //
    // The tone follows the caveat: `info` while there is only a rewrite to
    // report (aliases doing their job is not a problem), `warning` once the two
    // tools disagree. `aliasEffectTone` owns that, as it owns every other
    // decision here — this file is markup.
    //
    // # The layout
    //
    // `flex` before `flex-col` is load-bearing. daisyUI's `.alert` is
    // `display:grid; grid-auto-flow:column`, so `flex-col` alone is inert and
    // every child becomes its own narrow column — a headline, a list of renames
    // and an explanatory paragraph side by side in thirds. That is the
    // "columnar structure … tiny thin columns" this panel was reported for, and
    // `routes/alertStacking.test.ts` now fails on it everywhere.
    import {
        aliasEffectTone,
        aliasNotice,
        aliasText,
        canInstallParityFix,
        PARITY_DIFFERENCE_LEAD,
        PARITY_EXPLAINER,
        PARITY_SAME_ACCOUNTS,
        parityFixLabel,
        parityNotice,
        parityRepeatsRenames,
        parityWarning,
        relevantAliases,
        showsAliasEffect,
    } from "../aliasModel";
    import type {AliasEffect, AliasEntry, ConfWritten} from "../importTypes";
    import RenameList from "./RenameList.svelte";

    let {
        effect,
        aliases,
        editable,
        confWriting,
        confWritten,
        confError,
        onInstallConf,
    }: {
        /** The engine's MEASUREMENT of what the aliases did, or null when none is in force. */
        effect: AliasEffect | null;
        /** The journal's aliases, so a rename can be attributed to the line that caused it. */
        aliases: readonly AliasEntry[];
        /** A journal is bound to an editor, so the config fix could be written at all. */
        editable: boolean;
        confWriting: boolean;
        confWritten: ConfWritten | null;
        confError: string | null;
        onInstallConf: (revision: string) => void;
    } = $props();

    const tone = $derived(aliasEffectTone(effect));
    /** The rewrite half. Null when the aliases matched nothing here, which is the ordinary import. */
    const rewrite = $derived(aliasNotice(effect));
    const parityHeadline = $derived(parityNotice(effect));
</script>

{#if effect !== null && showsAliasEffect(effect)}
    <div
        class="alert flex flex-col items-start gap-3 rounded-box py-3 text-sm {tone === 'warning' ? 'alert-warning' : 'alert-info'}"
        role={tone === "warning" ? "alert" : "status"}
        data-testid="imports-alias-effect"
    >
        {#if rewrite !== null}
            <!--
                An account rewrite happening silently, immediately before the
                only irreversible step on this screen, is exactly what must not
                happen. The renames are the ENGINE's measurement — the same
                import run again with no aliases, diffed — so this is what
                hledger will do rather than what we think it will.
            -->
            <div class="flex w-full flex-col gap-2">
                <p class="font-semibold">{rewrite}</p>
                <RenameList renames={effect.renames} testid="imports-alias-renames" />
                {#each relevantAliases(aliases, effect) as relevance (relevance.alias.journalId + relevance.alias.index)}
                    <p class="text-xs">
                        {relevance.attributable ? "From" : "Possibly from"}
                        <code class="break-all">{aliasText(relevance.alias)}</code>
                        in {relevance.alias.journalId}, line {relevance.alias.line}.
                    </p>
                {/each}
            </div>
        {/if}

        {#if parityHeadline !== null}
            {@const parity = effect.cli}
            <!--
                Command-line parity. An `alias` line in a journal is not applied
                to an imported CSV — hledger's behaviour, not ours — so Ledgeline
                forwards it explicitly and a plain `hledger import` in a terminal
                does not. That means the same statement, the same rules file and
                the same journal produce two different sets of account names
                depending on which tool was reached for, silently.

                `parityNotice` is null whenever the engine MEASURED the two as
                agreeing, which is every ordinary import, so this half is
                invisible until it is the answer to a real question.

                Subordinated to the rewrite above by a rule and a smaller
                headline — but only when there IS a rewrite above it. When this
                is the whole panel it is the panel's primary statement and is
                not indented under nothing.
            -->
            <div class="flex w-full flex-col gap-2 {rewrite === null ? '' : 'border-t border-current/20 pt-3'}" data-testid="imports-cli-parity">
                <p class="text-xs font-semibold">{parityHeadline}</p>
                <p class="text-xs">{PARITY_EXPLAINER}</p>

                <!-- The duplicate list, and the whole of "much the same thing".
                     With no config file in force these differences ARE the
                     renames above, pair for pair — so the old screen printed
                     one list twice. Name them instead. -->
                {#if parityRepeatsRenames(effect)}
                    <p class="text-xs" data-testid="imports-cli-parity-same">{PARITY_SAME_ACCOUNTS}</p>
                {:else if parity.differences.length > 0}
                    <p class="text-xs">{PARITY_DIFFERENCE_LEAD}</p>
                    <RenameList renames={parity.differences} testid="imports-cli-parity-differences" />
                {/if}

                {#if parityWarning(parity) !== null}
                    <p class="text-xs" data-testid="imports-cli-parity-warning">{parityWarning(parity)}</p>
                {/if}

                {#if parity.additions.length > 0}
                    <!-- Shown BEFORE the button is pressed, because the
                         conversion is not lossless: a space becomes `.`, which
                         matches any character, and a plain alias written as a
                         regex becomes case-insensitive. Both are widenings, and
                         a user is entitled to see the exact line before it is
                         written. -->
                    <p class="text-xs">These lines would be added:</p>
                    <ul class="flex flex-col gap-1" data-testid="imports-cli-parity-additions">
                        {#each parity.additions as addition (addition)}
                            <li class="font-mono text-xs break-all">--alias={addition}</li>
                        {/each}
                    </ul>
                {/if}

                {#each parity.refusals as refusal (refusal.pattern + refusal.replacement)}
                    <p class="text-xs" data-testid="imports-cli-parity-refusal">
                        <code class="break-all">{refusal.pattern} → {refusal.replacement}</code> cannot be added because {refusal.message}.
                    </p>
                {/each}

                {#if canInstallParityFix(parity, editable)}
                    <button
                        type="button"
                        class="btn btn-sm"
                        disabled={confWriting}
                        onclick={() => onInstallConf(parity.revision)}
                        data-testid="imports-cli-parity-fix"
                    >
                        {#if confWriting}<span class="loading loading-xs loading-spinner"></span>{/if}
                        {parityFixLabel(parity)}
                    </button>
                {/if}

                {#if confWritten !== null}
                    <p class="text-xs" data-testid="imports-cli-parity-written">
                        {confWritten.created ? "Created" : "Updated"}
                        {confWritten.confPath} with {confWritten.added.length} alias{confWritten.added.length === 1 ? "" : "es"}.
                    </p>
                {/if}
                {#if confError !== null}
                    <p class="text-xs text-error" data-testid="imports-cli-parity-error">{confError}</p>
                {/if}
            </div>
        {/if}
    </div>
{/if}
