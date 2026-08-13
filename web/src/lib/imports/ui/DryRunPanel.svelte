<script lang="ts">
    // The dry run: everything that must be seen before anything is written.
    //
    // Four things in one panel, in the order they matter:
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
    //  3. The balance reconciliation — statement vs computed vs the difference.
    //  4. The git block. A modified target refuses the import until it is
    //     committed, and the ENGINE enforces that too; this panel is not the
    //     only thing standing between an import and an unrecoverable overwrite.
    //
    // The proposed entries are hledger's stdout — valid, re-parseable journal
    // text, not scraped human-readable output — so they are shown as they are.
    import AsyncSection from "$lib/components/AsyncSection.svelte";
    import {aliasNotice, aliasText, relevantAliases, renameText} from "../aliasModel";
    import {balanceVerdict, canWrite, gitBlockMessage, skippedWarning} from "../importModel";
    import type {AliasEntry, DryRunResult} from "../importTypes";

    let {
        view,
        result,
        error,
        aliases,
        writing,
        onRetry,
        onWrite,
    }: {
        view: import("$lib/stores/loadState").DataView;
        result: DryRunResult | null;
        error: Error | null;
        /** The journal's aliases, so a rename can be shown beside the line that caused it. */
        aliases: readonly AliasEntry[];
        /** The real import is running, so `Write changes` must not be pressable twice. */
        writing: boolean;
        onRetry: () => void;
        onWrite: () => void;
    } = $props();
</script>

<AsyncSection {view} value={result} {error} testid="imports-dry-run-error" label="the dry run" loadingLabel="Running the import as a dry run" {onRetry}>
    {#snippet children(run)}
        <section class="border-base-content/10 rounded-box flex flex-col gap-3 border p-3" aria-label="Dry run" data-testid="imports-dry-run">
            {#if !run.ok}
                <h2 class="text-error text-sm font-semibold tracking-tight">hledger refused this import.</h2>
                <p class="text-base-content/60 text-xs">This is hledger's own output, unedited — it usually names the row it choked on.</p>
                <!-- VERBATIM. See (1) at the head of this file. -->
                <pre
                    class="bg-base-300 rounded-box max-h-96 overflow-auto p-3 text-xs whitespace-pre-wrap"
                    data-testid="imports-dry-run-stderr">{run.stderr}</pre>
            {:else}
                <h2 class="text-sm font-semibold tracking-tight">Nothing has been written yet</h2>
                <p class="font-mono text-xs" data-testid="imports-dry-run-status">{run.status}</p>

                {#if skippedWarning(run.skipped) !== null}
                    <div class="alert alert-warning rounded-box items-start py-2 text-sm" role="alert" data-testid="imports-skipped">
                        <span>{skippedWarning(run.skipped)}</span>
                    </div>
                {/if}

                <!--
                    An account rewrite happening silently, immediately before the
                    only irreversible step on this screen, is exactly what must
                    not happen. The renames are the ENGINE's measurement — the
                    same import run again with no aliases, diffed — so this is
                    what hledger will do rather than what we think it will.
                    `aliasNotice` returns null when the aliases matched nothing
                    here, which is what keeps the section quiet on the ordinary
                    import.
                -->
                {#if aliasNotice(run.aliases) !== null}
                    <div class="alert alert-info rounded-box flex-col items-start gap-2 py-2 text-sm" role="status" data-testid="imports-alias-effect">
                        <span class="font-semibold">{aliasNotice(run.aliases)}</span>
                        <ul class="list-inside list-disc font-mono text-xs">
                            {#each run.aliases?.renames ?? [] as rename (rename.from)}
                                <li>{renameText(rename)}</li>
                            {/each}
                        </ul>
                        {#each relevantAliases(aliases, run.aliases) as relevance (relevance.alias.journalId + relevance.alias.index)}
                            <span class="text-xs">
                                {relevance.attributable ? "From" : "Possibly from"}
                                <code>{aliasText(relevance.alias)}</code>
                                in {relevance.alias.journalId}, line {relevance.alias.line}.
                            </span>
                        {/each}
                    </div>
                {/if}

                {#if run.balance !== null}
                    {@const verdict = balanceVerdict(run.balance)}
                    <div
                        class="alert rounded-box flex-col items-start gap-1 py-2 text-sm {verdict.tone === 'success' ? 'alert-success' : 'alert-error'}"
                        role="status"
                        data-testid="imports-balance-check"
                    >
                        <span class="font-semibold">{verdict.headline}</span>
                        <span>{verdict.detail}</span>
                    </div>
                {/if}

                {#if gitBlockMessage(run.blockedByGit) !== null}
                    <div class="alert alert-warning rounded-box flex-col items-start gap-2 py-2 text-sm" role="alert" data-testid="imports-git-blocked">
                        <span>{gitBlockMessage(run.blockedByGit)}</span>
                        <ul class="list-inside list-disc font-mono text-xs">
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
                    <pre class="bg-base-300 rounded-box max-h-96 overflow-auto p-3 text-xs" data-testid="imports-dry-run-entries">{run.entries}</pre>
                </div>

                <div class="flex items-center gap-2">
                    <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        disabled={!canWrite(run) || writing}
                        onclick={onWrite}
                        data-testid="imports-write-changes"
                    >
                        {#if writing}<span class="loading loading-spinner loading-xs"></span>{/if}
                        Write changes
                    </button>
                    {#if run.count === 0}
                        <span class="text-base-content/60 text-xs">There is nothing new to import — every row is already in your journal.</span>
                    {/if}
                </div>
            {/if}
        </section>
    {/snippet}
</AsyncSection>
