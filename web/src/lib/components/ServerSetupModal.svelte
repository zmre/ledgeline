<script lang="ts">
    // First-run server setup (WP-02): verify the hledger-web URL via GET /version
    // and persist it. On network/CORS failure, show the exact launch command to copy.
    import {ApiUnreachableError} from "$lib/api/client";
    import {settings} from "$lib/stores/settings.svelte";

    let url = $state("http://127.0.0.1:5000");
    let verifying = $state(false);
    let errorMessage = $state<string | null>(null);
    let unreachable = $state(false);
    let copied = $state(false);

    const launchCommand = "hledger-web -f YOUR.journal --serve-api --cors='*' --allow=view";

    async function verify(event: SubmitEvent): Promise<void> {
        event.preventDefault();
        verifying = true;
        errorMessage = null;
        unreachable = false;
        try {
            await settings.setServerUrl(url);
        } catch (error) {
            unreachable = error instanceof ApiUnreachableError;
            errorMessage = error instanceof Error ? error.message : String(error);
        } finally {
            verifying = false;
        }
    }

    async function copyCommand(): Promise<void> {
        try {
            await navigator.clipboard.writeText(launchCommand);
            copied = true;
            setTimeout(() => (copied = false), 1500);
        } catch {
            // Clipboard unavailable (e.g. insecure context); the command stays selectable.
        }
    }
</script>

<div class="modal modal-open" role="dialog" aria-labelledby="server-setup-title" aria-modal="true">
    <div class="modal-box max-w-lg">
        <h3 id="server-setup-title" class="text-lg font-bold">Connect to hledger-web</h3>
        <p class="text-base-content/70 py-2 text-sm">
            Ledgeline reads your journal from a locally running <code>hledger-web</code> JSON API. Enter its URL to get started.
        </p>
        <form onsubmit={verify}>
            <label class="form-control w-full">
                <span class="label-text pb-1 text-sm">Server URL</span>
                <input
                    type="url"
                    class="input input-bordered w-full"
                    bind:value={url}
                    placeholder="http://127.0.0.1:5000"
                    required
                    autocomplete="url"
                    disabled={verifying}
                />
            </label>

            {#if errorMessage !== null}
                <div class="alert alert-error mt-3 text-sm" role="alert">
                    <span>{errorMessage}</span>
                </div>
                {#if unreachable}
                    <div class="bg-base-200 mt-3 rounded-lg p-3">
                        <p class="text-base-content/70 pb-2 text-sm">Is hledger-web running? Launch it with CORS enabled:</p>
                        <div class="flex items-center gap-2">
                            <code class="bg-base-300 grow overflow-x-auto rounded p-2 text-xs whitespace-nowrap select-all">{launchCommand}</code>
                            <button type="button" class="btn btn-sm shrink-0" onclick={copyCommand}>
                                {copied ? "Copied!" : "Copy"}
                            </button>
                        </div>
                    </div>
                {/if}
            {/if}

            <div class="modal-action">
                <button type="submit" class="btn btn-primary" disabled={verifying || url.trim() === ""}>
                    {#if verifying}
                        <span class="loading loading-spinner loading-sm"></span>
                        Verifying…
                    {:else}
                        Verify &amp; connect
                    {/if}
                </button>
            </div>
        </form>
    </div>
</div>
