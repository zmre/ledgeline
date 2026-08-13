<script lang="ts">
    // Missing or too-old hledger — and the control that fixes it.
    //
    // This is the first screen a new user hits, and it gates everything else:
    // importing shells out to hledger, so with none resolved there is nothing
    // the rest of the page could honestly offer. The definition of done for this
    // work package names it explicitly — "never a stack trace or a silent
    // failure" — so the banner carries three things: what is wrong in our words,
    // the engine's own sentence underneath, and a path field that writes
    // `Prefs.hledgerPath` and re-probes.
    //
    // The path is validated by the ENGINE at store time (a non-executable is a
    // 400, not a value that persists and fails on the next import), so a
    // rejection here is the server's message shown verbatim.
    import {hledgerBannerCopy} from "../importModel";
    import type {ImportCapabilities} from "../importTypes";

    let {
        capabilities,
        initialPath,
        saving,
        error,
        onSave,
        onRecheck,
    }: {
        capabilities: ImportCapabilities;
        /** `prefs.hledgerPath`, so the field starts where the user left it. */
        initialPath: string | null;
        saving: boolean;
        /** The engine's refusal, verbatim. Null when nothing has been rejected. */
        error: string | null;
        onSave: (path: string) => void;
        onRecheck: () => void;
    } = $props();

    const copy = $derived(hledgerBannerCopy(capabilities));
    /**
     * What the user has typed, or null while they have not.
     *
     * NOT `let path = $state(initialPath ?? "")`: the preferences blob is
     * fetched, and the capabilities probe that reveals this banner can answer
     * first, so a snapshot taken at mount would leave the field permanently
     * empty on exactly the reload where a path was already stored. Deriving
     * until the first keystroke lets the prefs land late and still prefill,
     * without ever overwriting what is being typed.
     */
    let typed = $state<string | null>(null);
    const path = $derived(typed ?? initialPath ?? "");
</script>

<section class="alert alert-error rounded-box flex flex-col items-start gap-3 py-3 text-sm" role="alert" data-testid="imports-hledger-missing">
    <div class="flex flex-col gap-1">
        <span class="font-semibold">{copy.headline}</span>
        <span>{copy.detail}</span>
        {#if capabilities.hledger.message !== null}
            <span class="font-mono text-xs opacity-80">{capabilities.hledger.message}</span>
        {/if}
    </div>

    {#if copy.offersPath}
        <div class="flex w-full flex-wrap items-end gap-2">
            <label class="form-control grow">
                <span class="label-text text-xs">Path to hledger</span>
                <input
                    type="text"
                    class="input input-sm w-full font-mono"
                    value={path}
                    placeholder="/usr/local/bin/hledger"
                    spellcheck="false"
                    autocomplete="off"
                    data-testid="imports-hledger-path"
                    oninput={(event) => (typed = event.currentTarget.value)}
                />
            </label>
            <button type="button" class="btn btn-sm" disabled={saving} onclick={() => onSave(path)} data-testid="imports-hledger-save">
                {#if saving}<span class="loading loading-spinner loading-xs"></span>{/if}
                Use this
            </button>
            <button type="button" class="btn btn-ghost btn-sm" disabled={saving} onclick={onRecheck}>Check again</button>
        </div>
        <p class="text-xs opacity-70">
            Leave it empty to go back to looking on your <code>PATH</code>.
        </p>
    {/if}

    {#if error !== null}
        <p class="font-mono text-xs" data-testid="imports-prefs-error">{error}</p>
    {/if}
</section>
