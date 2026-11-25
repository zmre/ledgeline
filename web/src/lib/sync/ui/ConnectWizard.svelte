<!-- The Connect-a-bank wizard (plan 43 §4.3): a modal with a step rail —
     choose, SimpleFIN setup, link accounts, first sync preview — and Back on
     every step. Nothing is written until Finish (claiming a token stores the
     access key; that is SimpleFIN's side, not the journal's).

     It opens at "choose" from + Connect a bank, at "simplefin" for Replace
     token / Reconnect, and at "link" for Link… and Re-check accounts. The
     step logic is `wizardModel.ts`; this file renders it and talks to the
     store. Each step's heading takes focus when the step appears, so a
     keyboard or screen-reader user lands on the new content. -->
<script lang="ts">
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {tick, untrack} from "svelte";
    import {importStore} from "#lib/imports/importStore.svelte.js";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {SIMPLEFIN_URL, type ProblemEffect} from "../problemAction";
    import {runToast} from "../statusLine";
    import {syncStore, type SyncFailure} from "../syncStore.svelte";
    import type {WireRemoteAccount} from "../types";
    import {
        checkSetupToken,
        draftFor,
        linkRequests,
        looksLikeSetupToken,
        previewLine,
        previousStep,
        railSteps,
        STEP_LABELS,
        STEP_ORDER,
        wizardInit,
        wizardReduce,
        type ConnectMethod,
        type LinkDraft,
        type WizardStep,
    } from "../wizardModel";
    import LinkAccountForm from "./LinkAccountForm.svelte";
    import ProblemNotice from "./ProblemNotice.svelte";
    import SharePreviewDialog from "./SharePreviewDialog.svelte";
    import WhoSeesData from "./WhoSeesData.svelte";

    let {
        start = "choose",
        connectionId = null,
        accounts = [],
        onClose,
    }: {
        start?: WizardStep;
        /** The connection a Replace token or Link… is about. */
        connectionId?: string | null;
        /** The accounts a Link… or Re-check opens on. */
        accounts?: readonly WireRemoteAccount[];
        /** Closed; `highlightManual` when "I'll download statements myself" was the answer. */
        onClose: (result: {highlightManual: boolean}) => void;
    } = $props();

    const headingId = $props.id();
    const today = localToday();

    let wizard = $state.raw(untrack(() => wizardInit(start, connectionId, accounts)));
    /** Replacing a token keeps the connection's id and links. */
    const replacing = untrack(() => (start === "simplefin" ? connectionId : null));
    /** Adding this computer to a connection another computer already syncs. */
    const alsoHere = untrack(() => replacing !== null && syncStore.status?.connections.find((c) => c.id === replacing)?.state === "elsewhere");

    let method = $state<ConnectMethod>("simplefin");
    let token = $state("");
    let clipboardToken = $state<string | null>(null);
    let busy = $state(false);
    let failure = $state.raw<SyncFailure | null>(null);
    let issues = $state<string[]>([]);
    let drafts = $state<LinkDraft[]>([]);
    let sharing = $state<number | null>(null);
    let keepUpToDate = $state(settings.autoSync);

    const check = $derived(checkSetupToken(token));
    const defaultTarget = $derived(importStore.capabilities?.journals.find((j) => j.isRoot)?.id ?? "");
    const journals = $derived(importStore.capabilities?.journals ?? []);

    function dispatch(event: Parameters<typeof wizardReduce>[1]): void {
        wizard = wizardReduce(wizard, event);
        failure = null;
        issues = [];
        if (wizard.closed !== null) onClose(wizard.closed);
    }

    // A draft for every account the link step shows, seeded once each.
    $effect(() => {
        const remotes = wizard.accounts;
        untrack(() => {
            const have = new Set(drafts.map((d) => d.remoteId));
            const fresh = remotes.filter((r) => !have.has(r.remoteId)).map((r) => draftFor(r, defaultTarget, today));
            if (fresh.length > 0) drafts = [...drafts, ...fresh];
        });
    });

    // Focus the new step's heading.
    let heading: HTMLElement | undefined = $state();
    $effect(() => {
        void wizard.step;
        void tick().then(() => heading?.focus());
    });

    function close(): void {
        if (busy) return;
        onClose({highlightManual: false});
    }

    /**
     * "Paste token from clipboard" (contract WP-W): the clipboard is read only
     * when the field or the window gains focus — never on a timer — and its
     * text goes into the field only when the button is pressed.
     */
    async function offerClipboard(): Promise<void> {
        if (wizard.step !== "simplefin" || token.trim() !== "" || navigator.clipboard?.readText === undefined) return;
        try {
            const text = (await navigator.clipboard.readText()).trim();
            clipboardToken = looksLikeSetupToken(text) ? text : null;
        } catch {
            clipboardToken = null;
        }
    }

    async function connect(): Promise<void> {
        if (check.state !== "ok" || busy) return;
        busy = true;
        failure = null;
        const outcome = await syncStore.connect(token.trim(), replacing);
        busy = false;
        if (!outcome.ok) {
            failure = outcome.failure;
            return;
        }
        token = "";
        dispatch({type: "claimed", connectionId: outcome.value.connectionId, accounts: outcome.value.accounts});
    }

    async function recheck(): Promise<void> {
        if (wizard.connectionId === null) return;
        busy = true;
        const outcome = await syncStore.recheckAccounts(wizard.connectionId);
        busy = false;
        if (outcome.ok) dispatch({type: "accounts", accounts: outcome.value});
        else failure = outcome.failure;
    }

    const shown = $derived(drafts.filter((d) => wizard.accounts.some((a) => a.remoteId === d.remoteId)));

    async function preview(): Promise<void> {
        const built = linkRequests(shown, wizard.accounts);
        issues = built.issues;
        if (built.issues.length > 0) return;
        busy = true;
        const outcome = await syncStore.linkPreview(built.requests);
        busy = false;
        if (outcome.ok) dispatch({type: "previewed", previews: outcome.value});
        else failure = outcome.failure;
    }

    async function finish(): Promise<void> {
        const built = linkRequests(shown, wizard.accounts);
        if (built.issues.length > 0) {
            issues = built.issues;
            return;
        }
        busy = true;
        const outcome = await syncStore.link(built.requests);
        busy = false;
        if (!outcome.ok) {
            failure = outcome.failure;
            return;
        }
        if (keepUpToDate !== settings.autoSync) settings.autoSync = keepUpToDate;
        dispatch({type: "finished", run: outcome.value});
    }

    async function reviewNow(): Promise<void> {
        onClose({highlightManual: false});
        reviewLaunch.open("recents");
        await goto(resolve("/"));
    }

    function onEffect(effect: ProblemEffect): void {
        if (effect.type === "url") window.open(effect.url, "_blank", "noopener,noreferrer");
        else if (effect.type === "sync") void connect();
    }

    const rail = $derived(railSteps(wizard));
    const stepAt = $derived(STEP_ORDER.indexOf(wizard.step === "done" ? "preview" : wizard.step));
    const back = $derived(previousStep(wizard));
    const remoteById = $derived(new Map(wizard.accounts.map((a) => [a.remoteId, a])));
    const doneToast = $derived(wizard.run === null ? null : runToast(wizard.run));
</script>

<!-- The window regaining focus is the other moment the clipboard may be read: the user has just copied the token on SimpleFIN. -->
<svelte:window onfocus={() => void offerClipboard()} />

<div
    class="modal modal-open"
    role="dialog"
    aria-modal="true"
    aria-labelledby={headingId}
    tabindex="-1"
    data-testid="sync-wizard"
    data-step={wizard.step}
    use:dismissible={{trap: true, onDismiss: close, active: sharing === null}}
>
    <div class="modal-box flex max-h-[92vh] max-w-3xl flex-col gap-4 overflow-y-auto">
        <header class="flex flex-col gap-3">
            <h2 id={headingId} class="sr-only">Connect a bank</h2>
            <ul class="steps steps-horizontal w-full text-xs" aria-label="Steps">
                {#each rail as step (step)}
                    <li class="step {STEP_ORDER.indexOf(step) <= stepAt ? 'step-primary' : ''}" aria-current={step === wizard.step ? "step" : undefined}>
                        {STEP_LABELS[step]}
                    </li>
                {/each}
            </ul>
        </header>

        {#if wizard.step === "choose"}
            <section class="flex flex-col gap-3">
                <h3 class="text-base font-semibold" tabindex="-1" bind:this={heading}>How should Ledgeline get your transactions?</h3>
                <label class="flex cursor-pointer gap-3 rounded-box border border-base-content/10 p-3 {method === 'simplefin' ? 'border-primary' : ''}">
                    <input type="radio" class="radio mt-1 radio-sm" name="{headingId}-method" value="simplefin" bind:group={method} />
                    <span class="flex flex-col gap-1">
                        <span class="flex flex-wrap justify-between gap-2 font-medium">
                            Through SimpleFIN Bridge <span class="text-xs font-normal text-base-content/60">Third party · paid subscription</span>
                        </span>
                        <span class="text-sm text-base-content/80">
                            Works with most US banks. SimpleFIN and its data provider MX can see your transactions; Ledgeline never sees your bank password.
                            You'll set it up on SimpleFIN's website, then paste a token here.
                        </span>
                    </span>
                </label>
                <label class="flex cursor-pointer gap-3 rounded-box border border-base-content/10 p-3 {method === 'manual' ? 'border-primary' : ''}">
                    <input type="radio" class="radio mt-1 radio-sm" name="{headingId}-method" value="manual" bind:group={method} />
                    <span class="flex flex-col gap-1">
                        <span class="flex flex-wrap justify-between gap-2 font-medium">
                            I'll download statements myself <span class="text-xs font-normal text-base-content/60">Nobody else involved</span>
                        </span>
                        <span class="text-sm text-base-content/80">Always works. Ledgeline reminds you when an account is getting stale.</span>
                    </span>
                </label>
                <WhoSeesData summary="Why the difference?" />
            </section>
        {:else if wizard.step === "simplefin"}
            <section class="flex flex-col gap-3">
                <div class="flex flex-wrap items-baseline justify-between gap-2">
                    <h3 class="text-base font-semibold" tabindex="-1" bind:this={heading}>
                        {replacing === null ? "Set up SimpleFIN Bridge" : alsoHere ? "Sync from this computer too" : "Paste a new setup token"}
                    </h3>
                    {#if replacing === null}<span class="text-xs text-base-content/60">About 5 minutes</span>{/if}
                </div>
                {#if alsoHere}
                    <div class="flex flex-col gap-1 rounded-box bg-base-200 p-3 text-sm" data-testid="sync-also-here">
                        <p>Another computer already syncs these accounts. You don't need to connect this one to see what it imports.</p>
                        <p>
                            If both computers sync, they can write the same transactions at nearly the same time, before iCloud (or whatever shares the ledger)
                            has copied the other's changes across. That can duplicate transactions or leave a conflicted copy of a file. Turn on automatic sync
                            on one computer only, as that computer's own setting rather than this ledger's, and use
                            <strong>Sync now</strong> here only when the other one is off.
                        </p>
                        <p>Each computer needs its own setup token: a token works once.</p>
                    </div>
                {/if}
                <ol class="flex list-decimal flex-col gap-2 pl-5 text-sm">
                    {#if replacing === null}
                        <li>
                            <span class="flex flex-wrap items-center justify-between gap-2">
                                Create a SimpleFIN Bridge account and subscribe.
                                <a class="btn btn-primary btn-sm" href={SIMPLEFIN_URL} target="_blank" rel="noopener noreferrer external">Open SimpleFIN ↗</a>
                            </span>
                        </li>
                        <li>
                            On SimpleFIN, choose "Connect an institution" and sign in to your bank. Do this for each bank you want. (You can add more later.)
                        </li>
                    {:else}
                        <li>
                            <span class="flex flex-wrap items-center justify-between gap-2">
                                Open SimpleFIN. Your linked accounts and their history stay as they are.
                                <a class="btn btn-primary btn-sm" href={SIMPLEFIN_URL} target="_blank" rel="noopener noreferrer external">Open SimpleFIN ↗</a>
                            </span>
                        </li>
                    {/if}
                    <li>On SimpleFIN, choose "New app connection", name it "Ledgeline", and copy the setup token it shows.</li>
                    <li class="flex flex-col gap-2">
                        <label for="{headingId}-token">Paste the token here:</label>
                        <textarea
                            id="{headingId}-token"
                            class="textarea w-full font-mono text-xs"
                            rows="3"
                            spellcheck="false"
                            autocomplete="off"
                            placeholder="aHR0cHM6Ly9iZXRhLWJyaWRnZS5zaW1wbGVmaW4ub3Jn..."
                            bind:value={token}
                            onfocus={() => void offerClipboard()}
                            disabled={busy}
                            data-testid="sync-token"></textarea>
                        {#if clipboardToken !== null && token.trim() === ""}
                            <button
                                type="button"
                                class="btn self-start btn-xs"
                                onclick={() => {
                                    token = clipboardToken ?? "";
                                    clipboardToken = null;
                                }}>Paste token from clipboard</button
                            >
                        {/if}
                        <div class="flex flex-wrap items-center justify-between gap-2" aria-live="polite">
                            {#if check.state === "ok"}
                                <span class="text-xs {check.bridge ? 'text-success' : 'text-warning'}" data-testid="sync-token-check">
                                    {check.bridge ? "✓ " : "⚠ "}{check.note}
                                </span>
                            {:else if check.state === "invalid"}
                                <span class="text-xs text-error" data-testid="sync-token-check">{check.message}</span>
                            {:else}
                                <span></span>
                            {/if}
                            <button type="button" class="btn btn-primary btn-sm" disabled={check.state !== "ok" || busy} onclick={() => void connect()}>
                                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                                Connect
                            </button>
                        </div>
                    </li>
                </ol>
                <p class="text-xs text-base-content/70">
                    A setup token works once. Ledgeline exchanges it for a private access key and keeps that in your keychain. You can revoke it on SimpleFIN
                    any time.
                </p>
            </section>
        {:else if wizard.step === "link"}
            <section class="flex flex-col gap-3">
                <h3 class="text-base font-semibold" tabindex="-1" bind:this={heading}>
                    {wizard.accounts.length === 0
                        ? "No accounts yet"
                        : `SimpleFIN found ${wizard.accounts.length} account${wizard.accounts.length === 1 ? "" : "s"}. Choose where each one goes.`}
                </h3>
                {#if wizard.accounts.length === 0}
                    <p class="text-sm">SimpleFIN has no banks connected yet. Connect one on SimpleFIN, then press Re-check.</p>
                    <div class="flex gap-2">
                        <a class="btn btn-primary btn-sm" href={SIMPLEFIN_URL} target="_blank" rel="noopener noreferrer external">Open SimpleFIN ↗</a>
                        <button type="button" class="btn btn-sm" disabled={busy || wizard.connectionId === null} onclick={() => void recheck()}>
                            {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                            Re-check
                        </button>
                    </div>
                {:else}
                    {#each drafts as draft, i (draft.remoteId)}
                        {@const remote = remoteById.get(draft.remoteId)}
                        {#if remote !== undefined}
                            <LinkAccountForm {remote} bind:draft={drafts[i]} {journals} accountNames={journal.accountNames} onShare={() => (sharing = i)} />
                        {/if}
                    {/each}
                {/if}
            </section>
        {:else if wizard.step === "preview"}
            <section class="flex flex-col gap-3">
                <h3 class="text-base font-semibold" tabindex="-1" bind:this={heading}>First sync</h3>
                <p class="text-sm text-base-content/70">This is what Finish will write. Nothing has been written yet.</p>
                {#each wizard.previews ?? [] as p (p.remoteId)}
                    <div class="flex flex-col gap-1 rounded-box border border-base-content/10 p-3" data-testid="sync-preview-row">
                        <span class="font-medium">{remoteById.get(p.remoteId)?.label ?? p.remoteId}</span>
                        {#if p.ok}
                            <span class="text-sm">{previewLine(p)}</span>
                            {#if p.entries !== ""}
                                <details class="text-sm">
                                    <summary class="cursor-pointer text-base-content/70">Show the transactions</summary>
                                    <pre class="mt-1 max-h-64 overflow-auto rounded-box bg-base-200 p-2 text-xs">{p.entries}</pre>
                                </details>
                            {/if}
                        {:else if p.error !== null}
                            <ProblemNotice problem={p.error} scope={{connectionId: wizard.connectionId, accountKey: null}} {onEffect} />
                        {/if}
                    </div>
                {/each}
                <label class="flex cursor-pointer items-start gap-3">
                    <input type="checkbox" class="toggle mt-0.5 toggle-primary" bind:checked={keepUpToDate} data-testid="sync-keep-up-to-date" />
                    <span class="flex flex-col">
                        <span>Keep these up to date automatically</span>
                        <span class="text-xs text-base-content/70">
                            While Ledgeline is open it syncs once each morning and about every {settings.autoSyncEveryHours} hours, and adds new transactions for
                            you to review.
                        </span>
                    </span>
                </label>
            </section>
        {:else}
            <section class="flex flex-col gap-3">
                <h3 class="text-base font-semibold" tabindex="-1" bind:this={heading}>Linked</h3>
                {#if doneToast !== null}<p class="text-sm" data-testid="sync-done-summary">{doneToast.text}</p>{/if}
                <p class="text-sm text-base-content/70">New transactions are in your journal, marked as not yet reviewed.</p>
            </section>
        {/if}

        {#if failure !== null}
            {#if failure.problem !== null}
                <ProblemNotice problem={failure.problem} scope={{connectionId: wizard.connectionId, accountKey: null}} {onEffect} />
            {:else}
                <p class="text-sm text-error" role="alert">{failure.message}</p>
            {/if}
        {/if}
        {#if issues.length > 0}
            <ul class="text-sm text-error" role="alert">
                {#each issues as issue (issue)}<li>{issue}</li>{/each}
            </ul>
        {/if}

        <footer class="modal-action mt-0 flex-wrap">
            {#if wizard.step === "done"}
                {#if (doneToast?.review ?? 0) > 0}
                    <button type="button" class="btn btn-primary btn-sm" onclick={() => void reviewNow()} data-testid="sync-review-now">Review now</button>
                {/if}
                <button type="button" class="btn btn-sm" onclick={() => onClose({highlightManual: false})}>Done</button>
            {:else}
                <button type="button" class="btn mr-auto btn-ghost btn-sm" disabled={busy} onclick={close}>Cancel</button>
                {#if back !== null}
                    <button type="button" class="btn btn-sm" disabled={busy} onclick={() => dispatch({type: "back"})}>Back</button>
                {/if}
                {#if wizard.step === "choose"}
                    <button type="button" class="btn btn-primary btn-sm" onclick={() => dispatch({type: "choose", method})}>
                        {method === "manual" ? "Show my accounts" : "Next"}
                    </button>
                {:else if wizard.step === "link" && wizard.accounts.length > 0}
                    <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => void preview()} data-testid="sync-preview">
                        {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Preview
                    </button>
                {:else if wizard.step === "preview"}
                    <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        disabled={busy || !(wizard.previews ?? []).some((p) => p.ok)}
                        onclick={() => void finish()}
                        data-testid="sync-finish"
                    >
                        {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                        Finish
                    </button>
                {/if}
            {/if}
        </footer>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Close" onclick={close}></button>
</div>

{#if sharing !== null}
    {@const draft = drafts[sharing]}
    {@const remote = draft === undefined ? undefined : remoteById.get(draft.remoteId)}
    {#if draft !== undefined && remote?.suggestion.manualRules}
        <SharePreviewDialog
            manualRulesId={remote.suggestion.manualRules.id}
            journalAccount={draft.createAccount ? draft.newAccount.name : draft.journalAccount}
            onConfirm={() => {
                if (sharing !== null) drafts[sharing].share = "share";
                sharing = null;
            }}
            onCancel={() => (sharing = null)}
        />
    {/if}
{/if}
