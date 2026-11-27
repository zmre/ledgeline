<!-- Rename an account and everything under it (T-accounts *Rename*): every
     matching line in every time file and the accounts file. First a preview
     lists each line it changes, before and after; nothing is written until
     Rename. Billed lines keep their tags. Changing the new name after a
     preview asks for a new one. -->
<script lang="ts">
    import {untrack} from "svelte";
    import type {RenamePreview} from "../api/accounts.js";
    import AccountDialogShell from "./AccountDialogShell.svelte";

    let {
        from,
        onPreview,
        onApply,
        onCancel,
    }: {
        from: string;
        /** The lines a rename to `to` changes, or why it can't. */
        onPreview: (to: string) => Promise<RenamePreview | string>;
        /** Rename to `to`; null when done, else why not. */
        onApply: (to: string) => Promise<string | null>;
        onCancel: () => void;
    } = $props();

    let to = $state(untrack(() => from));
    let preview = $state<RenamePreview | null>(null);
    let busy = $state(false);
    let error = $state<string | null>(null);

    const wanted = $derived(to.trim());
    const current = $derived(preview !== null && preview.to === wanted ? preview : null);

    async function submit(): Promise<void> {
        busy = true;
        error = null;
        if (current === null) {
            const answer = await onPreview(wanted);
            if (typeof answer === "string") error = answer;
            else preview = answer;
        } else {
            error = await onApply(wanted);
        }
        busy = false;
    }

    const lineCount = (n: number): string => `${n} ${n === 1 ? "line" : "lines"}`;
</script>

<AccountDialogShell
    title="Rename {from}"
    submitLabel={current === null ? "Preview" : `Rename ${lineCount(current.lines.length)}`}
    {busy}
    {error}
    submitDisabled={wanted === "" || wanted === from}
    onSubmit={() => void submit()}
    {onCancel}
    testid="account-rename-dialog"
>
    <label class="flex flex-col gap-1 text-sm"
        >New name
        <input class="input w-full input-sm" bind:value={to} autocomplete="off" spellcheck="false" data-initial data-testid="account-rename-to" />
    </label>
    {#if current !== null}
        <p class="text-sm" data-testid="account-rename-summary">
            {lineCount(current.lines.length)} in {current.files.length}
            {current.files.length === 1 ? "file" : "files"} change:
        </p>
        <div class="max-h-72 overflow-auto rounded-box border border-base-300">
            <table class="table table-xs" data-testid="account-rename-lines">
                <thead><tr><th>Where</th><th>Before</th><th>After</th></tr></thead>
                <tbody>
                    {#each current.lines as line (`${line.file}:${line.line}`)}
                        <tr data-testid="account-rename-line">
                            <td class="text-xs whitespace-nowrap text-base-content/60">{line.file}:{line.line}</td>
                            <td class="font-mono text-xs whitespace-pre">{line.before.trim()}</td>
                            <td class="font-mono text-xs whitespace-pre">{line.after.trim()}</td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {:else if preview !== null}
        <p class="text-sm text-base-content/70">The name changed since the preview; preview again.</p>
    {/if}
</AccountDialogShell>
