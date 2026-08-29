<script lang="ts">
    // The `?` sheet. Every row comes from `keymap.help`, which is the SAME
    // resolved list `keymap.handle` searches — so this cannot describe a keymap
    // the app does not have. That is the one structural guarantee of this
    // feature; `dispatch.test.ts` pins it with an explicit agreement test.
    //
    // It lists only what is active RIGHT NOW, not a full catalogue. The point of
    // context scoping is that `j` means different things in different places; a
    // sheet listing `j` twice with no way to tell which applies is worse than no
    // sheet, and a complete catalogue would need the parallel list this design
    // exists to avoid.
    import {dismissible} from "./dismissible";
    import {keymap} from "./keymap.svelte";

    // Structure copied from TransactionModal: `role="dialog" aria-modal="true"`
    // on a div plus the `<button class="modal-backdrop">` trick is already known
    // clean under svelte-check, and this is not the place to discover which
    // invented alternative is not.
</script>

{#if keymap.helpOpen}
    <div
        class="modal modal-open"
        role="dialog"
        aria-modal="true"
        aria-label="Keyboard shortcuts"
        data-testid="key-help"
        use:dismissible={{onDismiss: () => keymap.closeHelp(), trap: true}}
    >
        <div class="modal-box max-w-3xl">
            <h3 class="mb-3 text-lg font-semibold">Keyboard shortcuts</h3>
            <div class="grid grid-cols-1 gap-x-8 gap-y-4 sm:grid-cols-2">
                {#each keymap.help as section (section.group)}
                    <section>
                        <h4 class="pb-1 text-xs font-medium tracking-wide text-base-content/60 uppercase">{section.group}</h4>
                        <ul class="flex flex-col gap-1">
                            {#each section.rows as row (row.keys)}
                                <li class="flex items-baseline justify-between gap-4">
                                    <span class="text-sm">{row.label}</span>
                                    <span class="flex shrink-0 items-center gap-1">
                                        {#each row.tokens as token, at (at)}
                                            <kbd class="kbd kbd-sm">{token.text}</kbd>
                                        {/each}
                                    </span>
                                </li>
                            {/each}
                        </ul>
                    </section>
                {/each}
            </div>
            <p class="mt-4 text-xs text-base-content/60">Shortcuts are off while you're typing in a field.</p>
        </div>
        <button type="button" class="modal-backdrop" aria-label="Close" onclick={() => keymap.closeHelp()}>close</button>
    </div>
{/if}
