<script lang="ts">
    // Compact txn status badge (WP-03): cleared `*`, pending `!`, unmarked dot.
    import type {TxnStatus} from "$lib/domain/types";

    let {status}: {status: TxnStatus} = $props();
</script>

{#if status === "cleared"}
    <span class="badge badge-success badge-outline badge-xs font-mono" title="cleared">*</span>
{:else if status === "pending"}
    <span class="badge badge-warning badge-outline badge-xs font-mono" title="pending">!</span>
{:else}
    <span class="badge badge-ghost badge-xs font-mono" title="unmarked">&middot;</span>
{/if}
