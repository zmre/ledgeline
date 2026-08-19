<script lang="ts">
    // An armed chord prefix is the app's only modal state, and it MUST be
    // visible: without this, a swallowed keystroke after a stray `g` is
    // indistinguishable from a broken app.
    //
    // `aria-hidden` because this is a visual affordance for a sighted power user
    // mid-keystroke — echoing `g` into a live region would be noise, and the
    // prefix self-clears after CHORD_TIMEOUT_MS anyway.
    import {formatKeys} from "./chord";
    import {keymap} from "./keymap.svelte";

    const tokens = $derived(keymap.pending === "" ? [] : formatKeys(keymap.pending));
</script>

{#if tokens.length > 0}
    <div class="pointer-events-none fixed bottom-4 left-4 z-40 flex items-center gap-1" aria-hidden="true" data-testid="chord-indicator">
        {#each tokens as token, at (at)}
            <kbd class="kbd kbd-sm">{token.text}</kbd>
        {/each}
    </div>
{/if}
