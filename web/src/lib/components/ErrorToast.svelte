<!-- The corner "the journal feed failed" toast, written once instead of three
     byte-identical times (journal / reports / holdings routes).

     This is the STALE-data channel: it reports that a refresh failed while
     something usable is still on screen. When a load fails with nothing to show
     at all, the page says so in full, in place — see the `loadFailed` branch on
     the journal route — and passes `null` here so this does not repeat it.

     `message` is `string | null` rather than `Error | null` because the journal
     store still carries its error as a string, unlike the report stores. -->
<script lang="ts">
    let {message, onRetry}: {message: string | null; onRetry: () => void} = $props();
</script>

{#if message !== null}
    <div class="toast toast-end z-30">
        <div class="alert alert-error">
            <span class="max-w-xs truncate" title={message}>{message}</span>
            <button type="button" class="btn btn-sm" onclick={onRetry}>Retry</button>
        </div>
    </div>
{/if}
