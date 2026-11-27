<!-- Setting up time tracking (plan 46 T2, T15, P-person): the first visit to
     Track before the time files exist. Two ways in:

     - Start fresh: your name (suggested from this computer's login, which
       you can change or clear: with no name, files aren't kept per person),
       the files that will be created (asked of the server without writing,
       again when the name changes), then Set up.
     - Use existing…: a time file or journal you already keep (the ones found
       under the books folder are offered), checked first, then used.

     Nothing is written until a button says so. -->
<script lang="ts">
    import {untrack} from "svelte";
    import type {AdoptAnswer, SetupFile, TrackApi} from "#lib/time/api/track.js";
    import type {SetupOffer} from "#lib/time/types.js";
    import {failureText} from "./tracker.svelte.js";

    let {offer, api, onDone, previewDelayMs = 250}: {offer: SetupOffer; api: TrackApi; onDone: () => void; previewDelayMs?: number} = $props();

    const uid = $props.id();
    let mode = $state<"create" | "adopt">("create");
    let person = $state(untrack(() => offer.suggested_person ?? ""));
    let files = $state<SetupFile[] | null>(null);
    let previewError = $state<string | null>(null);
    let busy = $state(false);
    let failure = $state<string | null>(null);

    let adoptFile = $state(untrack(() => offer.found[0] ?? ""));
    let adoptCheck = $state<AdoptAnswer | null>(null);
    let settingNeeded = $state<string | null>(null);

    /** What each file is for, in plain words. */
    function purpose(path: string): string {
        if (path === offer.file) return "Gathers all your time, for reports";
        if (path === offer.accounts_file) return "The accounts you track time against, starting with one per customer";
        return "Where this month's time is written";
    }

    // Ask what would be written, a moment after the name stops changing.
    let latest = 0;
    $effect(() => {
        const name = person;
        const mine = ++latest;
        const timer = setTimeout(() => {
            api.setup(name.trim(), true).then(
                (answer) => {
                    if (mine !== latest) return;
                    files = answer.files;
                    previewError = null;
                },
                (error: unknown) => {
                    if (mine !== latest) return;
                    files = null;
                    previewError = failureText(error);
                }
            );
        }, previewDelayMs);
        return () => clearTimeout(timer);
    });

    async function create(): Promise<void> {
        busy = true;
        failure = null;
        try {
            await api.setup(person.trim(), false);
            onDone();
        } catch (error) {
            failure = failureText(error);
        } finally {
            busy = false;
        }
    }

    async function check(): Promise<void> {
        const file = adoptFile.trim();
        if (file === "") return;
        busy = true;
        failure = null;
        adoptCheck = null;
        try {
            adoptCheck = await api.adopt(file, true);
        } catch (error) {
            failure = failureText(error);
        } finally {
            busy = false;
        }
    }

    async function adopt(): Promise<void> {
        if (adoptCheck === null) return;
        busy = true;
        failure = null;
        try {
            const done = await api.adopt(adoptCheck.file, false);
            if (done.needs_setting && !done.written) settingNeeded = done.file;
            else onDone();
        } catch (error) {
            failure = failureText(error);
        } finally {
            busy = false;
        }
    }
</script>

<section class="mx-auto flex w-full max-w-2xl flex-col gap-4" aria-labelledby="{uid}-title" data-testid="time-setup">
    <header class="flex flex-wrap items-start gap-3">
        <div class="min-w-0 flex-1">
            <h1 id="{uid}-title" class="text-xl font-semibold tracking-tight">Set up time tracking</h1>
            <p class="mt-1 text-sm text-base-content/70">
                Track the time you spend for each customer and bill it later. Your time is kept in plain text files beside your books, which hledger can read
                too.
            </p>
        </div>
        {#if mode === "create"}
            <button type="button" class="btn btn-primary" onclick={() => void create()} disabled={busy || files === null} data-testid="time-setup-create">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                Set up
            </button>
        {:else if adoptCheck !== null && settingNeeded === null}
            <button type="button" class="btn btn-primary" onclick={() => void adopt()} disabled={busy} data-testid="time-setup-adopt">Use this file</button>
        {/if}
    </header>

    <div role="tablist" class="tabs tabs-box w-fit tabs-sm" aria-label="How to start">
        <button
            type="button"
            role="tab"
            class="tab {mode === 'create' ? 'tab-active' : ''}"
            aria-selected={mode === "create"}
            onclick={() => (mode = "create")}
        >
            Start fresh
        </button>
        <button
            type="button"
            role="tab"
            class="tab {mode === 'adopt' ? 'tab-active' : ''}"
            aria-selected={mode === "adopt"}
            onclick={() => (mode = "adopt")}
            data-testid="time-setup-existing">Use existing…</button
        >
    </div>

    {#if mode === "create"}
        <div class="flex flex-col gap-1">
            <label class="text-sm font-medium" for="{uid}-person">Your name in the time files</label>
            <input id="{uid}-person" class="input w-full max-w-xs" type="text" autocomplete="off" bind:value={person} data-testid="time-setup-person" />
            <p class="text-xs text-base-content/70">
                A short name, so your time stays apart from anyone else's who tracks in these books. Leave it empty if you're the only one.
            </p>
        </div>
        <div class="flex flex-col gap-2">
            <h2 class="text-sm font-medium">These files will be created</h2>
            {#if previewError !== null}
                <div class="alert text-sm alert-warning" role="alert" data-testid="time-setup-preview-error">{previewError}</div>
            {:else if files === null}
                <p class="text-sm text-base-content/60">Checking…</p>
            {:else}
                <ul class="flex flex-col gap-2" data-testid="time-setup-files">
                    {#each files as file (file.path)}
                        <li class="rounded-box border border-base-300 px-3 py-2">
                            <div class="font-mono text-sm">{file.path}</div>
                            <div class="text-xs text-base-content/70">{purpose(file.path)}</div>
                            <details class="mt-1 text-xs">
                                <summary class="cursor-pointer text-base-content/60">Show what it will hold</summary>
                                <pre class="mt-1 overflow-x-auto rounded bg-base-200 p-2">{file.text}</pre>
                            </details>
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    {:else}
        <div class="flex flex-col gap-2">
            <p class="text-sm text-base-content/70">Already keep your time in a file hledger reads? Use it as it is; nothing else is created.</p>
            {#if offer.found.length > 0}
                <fieldset class="flex flex-col gap-1">
                    <legend class="mb-1 text-sm font-medium">Found in your books folder</legend>
                    {#each offer.found as found (found)}
                        <label class="flex items-center gap-2 text-sm">
                            <input
                                type="radio"
                                class="radio radio-sm"
                                name="{uid}-found"
                                value={found}
                                bind:group={adoptFile}
                                onchange={() => (adoptCheck = null)}
                            />
                            <span class="font-mono">{found}</span>
                        </label>
                    {/each}
                </fieldset>
            {/if}
            <label class="text-sm font-medium" for="{uid}-file">File</label>
            <div class="flex flex-wrap gap-2">
                <input
                    id="{uid}-file"
                    class="input min-w-0 flex-1"
                    type="text"
                    placeholder="time.journal"
                    autocomplete="off"
                    bind:value={adoptFile}
                    oninput={() => (adoptCheck = null)}
                    data-testid="time-setup-file"
                />
                <button type="button" class="btn" onclick={() => void check()} disabled={busy || adoptFile.trim() === ""} data-testid="time-setup-check"
                    >Check</button
                >
            </div>
            {#if adoptCheck !== null && settingNeeded === null}
                <p class="text-sm" role="status" data-testid="time-setup-checked">
                    {adoptCheck.file} can be used{adoptCheck.format === null ? "" : ` (${adoptCheck.format})`}.
                    {#if adoptCheck.needs_setting}Your books' settings will be updated to point at it.{/if}
                </p>
            {/if}
            {#if settingNeeded !== null}
                <div class="alert text-sm alert-info" role="status" data-testid="time-setup-setting">
                    <span>
                        One more step: your books have no settings file to record this in. Create <span class="font-mono">ledgeline.toml</span> beside your
                        journal with:
                        <pre class="mt-1">[time]
file = "{settingNeeded}"</pre>
                    </span>
                </div>
            {/if}
        </div>
    {/if}

    {#if failure !== null}
        <div class="alert text-sm alert-warning" role="alert" data-testid="time-setup-error">{failure}</div>
    {/if}
</section>
