<script lang="ts">
    // The dry run: everything that must be seen before anything is written.
    //
    // Five things in one panel, in the order they matter:
    //
    //  1. A FAILURE renders hledger's stderr VERBATIM in a `<pre>` and is never
    //     paraphrased. hledger's import errors echo the offending CSV record
    //     back, which is the single most useful thing that can be on this screen
    //     — it names the row that broke and usually the field. Summarising it
    //     ("the import failed") throws that away and leaves the user with no
    //     path forward at all.
    //  2. The `.latest` warning. hledger drops back-dated rows SILENTLY: state
    //     lives beside the data file, keyed to its name, holding the newest date
    //     already imported, and a row older than that simply is not in the
    //     output. Nothing in hledger's own text mentions it.
    //  3. What the journal's aliases do to these account names, and where that
    //     rewrite does not reach. ONE panel for both — `AliasEffectPanel` owns
    //     the whole of it, including the decision to stay quiet, which is what it
    //     does on the ordinary import. It was two alerts here and they were read
    //     as one thing said twice.
    //  4. The balance reconciliation — statement vs computed vs the difference.
    //  5. The git block. A modified target refuses the import until it is
    //     committed, and the ENGINE enforces that too; this panel is not the
    //     only thing standing between an import and an unrecoverable overwrite.
    //
    // The proposed entries are hledger's stdout — valid, re-parseable journal
    // text, not scraped human-readable output — so they are shown as they are.
    //
    // Every alert below says `flex flex-col`, never `flex-col` alone: daisyUI's
    // `.alert` is `display:grid; grid-auto-flow:column` and lays its children out
    // as narrow side-by-side columns without it. `routes/alertStacking.test.ts`
    // enforces that across every component.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {balanceVerdict, canWrite, gitBlockMessage, skippedWarning} from "../importModel";
    import type {AliasEntry, ConfWritten, DryRunResult} from "../importTypes";
    import AliasEffectPanel from "./AliasEffectPanel.svelte";

    let {
        view,
        result,
        error,
        aliases,
        writing,
        editable,
        confWriting,
        confWritten,
        confError,
        onRetry,
        onWrite,
        onInstallConf,
    }: {
        view: import("$lib/stores/loadState").DataView;
        result: DryRunResult | null;
        error: Error | null;
        /** The journal's aliases, so a rename can be shown beside the line that caused it. */
        aliases: readonly AliasEntry[];
        /** The real import is running, so `Write changes` must not be pressable twice. */
        writing: boolean;
        /** A journal is bound to an editor, so the engine will accept a write at all. */
        editable: boolean;
        /** The config-file fix is being written. */
        confWriting: boolean;
        confWritten: ConfWritten | null;
        confError: string | null;
        onRetry: () => void;
        onWrite: () => void;
        onInstallConf: (revision: string) => void;
    } = $props();
</script>

<AsyncSection {view} value={result} {error} testid="imports-dry-run-error" label="the dry run" loadingLabel="Running the import as a dry run" {onRetry}>
    {#snippet children(run)}
        <section class="flex flex-col gap-3 rounded-box border border-base-content/10 p-3" aria-label="Dry run" data-testid="imports-dry-run">
            {#if !run.ok}
                <h2 class="text-sm font-semibold tracking-tight text-error">hledger refused this import.</h2>
                <p class="text-xs text-base-content/60">This is hledger's own output, unedited — it usually names the row it choked on.</p>
                <!-- VERBATIM. See (1) at the head of this file. -->
                <pre
                    class="max-h-96 overflow-auto rounded-box bg-base-300 p-3 text-xs whitespace-pre-wrap"
                    data-testid="imports-dry-run-stderr">{run.stderr}</pre>
            {:else}
                <h2 class="text-sm font-semibold tracking-tight">Nothing has been written yet</h2>
                <p class="font-mono text-xs" data-testid="imports-dry-run-status">{run.status}</p>

                {#if skippedWarning(run.skipped) !== null}
                    <div class="alert items-start rounded-box py-2 text-sm alert-warning" role="alert" data-testid="imports-skipped">
                        <span>{skippedWarning(run.skipped)}</span>
                    </div>
                {/if}

                <!-- ONE panel for both alias facts — what the aliases rewrite
                     here, and where that rewrite does not reach. They were two
                     alerts and were read as one thing said twice;
                     `AliasEffectPanel` explains the merge and owns the whole of
                     it, including staying quiet on the ordinary import. -->
                <AliasEffectPanel effect={run.aliases} {aliases} {editable} {confWriting} {confWritten} {confError} {onInstallConf} />

                {#if run.balance !== null}
                    {@const verdict = balanceVerdict(run.balance)}
                    <!-- `flex` before `flex-col`: `.alert` is a grid with
                         `grid-auto-flow:column`, so without it the headline and
                         the three amounts under it sit in two columns.
                         See `routes/alertStacking.test.ts`. -->
                    <div
                        class="alert flex flex-col items-start gap-1 rounded-box py-2 text-sm {verdict.tone === 'success' ? 'alert-success' : 'alert-error'}"
                        role="status"
                        data-testid="imports-balance-check"
                    >
                        <span class="font-semibold">{verdict.headline}</span>
                        <span>{verdict.detail}</span>
                    </div>
                {/if}

                {#if gitBlockMessage(run.blockedByGit) !== null}
                    <!-- `flex` before `flex-col`: `.alert` is a grid with
                         `grid-auto-flow:column`, so without it the sentence and
                         the list of blocked paths become two thin columns.
                         See `routes/alertStacking.test.ts`. -->
                    <div class="alert flex flex-col items-start gap-2 rounded-box py-2 text-sm alert-warning" role="alert" data-testid="imports-git-blocked">
                        <span>{gitBlockMessage(run.blockedByGit)}</span>
                        <ul class="list-inside list-disc font-mono text-xs break-all">
                            {#each run.blockedByGit as path (path)}
                                <li>{path}</li>
                            {/each}
                        </ul>
                    </div>
                {/if}

                <div>
                    <h3 class="mb-1 text-xs font-semibold tracking-tight">
                        {run.count} transaction{run.count === 1 ? "" : "s"} would be added
                    </h3>
                    <pre class="max-h-96 overflow-auto rounded-box bg-base-300 p-3 text-xs" data-testid="imports-dry-run-entries">{run.entries}</pre>
                </div>

                <div class="flex items-center gap-2">
                    <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        disabled={!canWrite(run) || writing}
                        onclick={onWrite}
                        data-testid="imports-write-changes"
                    >
                        {#if writing}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Write changes
                    </button>
                    {#if run.count === 0}
                        <span class="text-xs text-base-content/60">There is nothing new to import — every row is already in your journal.</span>
                    {/if}
                </div>
            {/if}
        </section>
    {/snippet}
</AsyncSection>
