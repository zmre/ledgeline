<script lang="ts">
    // Compact txn status badge (WP-03): cleared `*`, pending `!`, unmarked dot.
    import type {TxnStatus} from "$lib/domain/types";

    let {status}: {status: TxnStatus} = $props();
</script>

{#if status === "cleared"}
    <span class="badge badge-outline font-mono badge-xs badge-success" title="cleared">*</span>
{:else if status === "pending"}
    <span class="badge badge-outline font-mono badge-xs badge-warning" title="pending">!</span>
{:else}
    <span class="badge badge-ghost font-mono badge-xs" title="unmarked">&middot;</span>
{/if}
