<script lang="ts">
    // Small corner icon when txn or posting comments exist (WP-03). Hover shows
    // the comment text in a daisyUI tooltip; the focusable button makes the
    // tooltip reachable by tap/keyboard on mobile.
    import type {Transaction} from "$lib/domain/types";
    import {txnComments} from "./rowModel";

    let {txn}: {txn: Transaction} = $props();

    const comments = $derived(txnComments(txn));
</script>

{#if comments.length > 0}
    <span class="tooltip tooltip-left shrink-0 before:max-w-64 before:whitespace-normal" data-tip={comments.join(" · ")}>
        <button type="button" class="text-base-content/50 hover:text-base-content flex cursor-help items-center align-middle" aria-label="Show comments">
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="h-3.5 w-3.5" aria-hidden="true">
                <path
                    fill-rule="evenodd"
                    d="M1 8c0-3.1 3.1-5.5 7-5.5s7 2.4 7 5.5-3.1 5.5-7 5.5c-.8 0-1.6-.1-2.3-.3L2.7 14.6a.5.5 0 0 1-.7-.5l.3-2.6C1.5 10.5 1 9.3 1 8Z"
                    clip-rule="evenodd"
                />
            </svg>
        </button>
    </span>
{/if}
