<script lang="ts">
    // A Find Matches card's footer (plan 35 D9), on plan 32's footer: one row
    // of fixed height, the positive action green at the bottom left, the
    // others grey at the right, every button showing its key.
    //
    //   clean   ✓ Link                     ⋯  ✕ Not a match   Skip →
    //   dirty   ✓ Link and Save   Revert   ⋯  ✕ Not a match   Skip →
    //
    // Dirty means a description was edited: Link and Not a match both save
    // the edits in the same write. ⋯ holds Ignore this side (one per side).
    // A combination card (Amendment A4) has no Not a match (a rejection is
    // pairwise) and one side to ignore: the single posting.
    import {dismissible} from "#lib/keys/dismissible.js";
    import {formatKeys} from "#lib/keys/chord.js";
    import {footerFade} from "#lib/review/ui/motion.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";

    let {
        dirty,
        reduced,
        busy = false,
        canReject = true,
        sides,
        onLink,
        onReject,
        onSkip,
        onRevert,
        onIgnore,
    }: {
        dirty: boolean;
        reduced: boolean;
        busy?: boolean;
        /** Not a match is offered (not on a combination). */
        canReject?: boolean;
        /** The halves' short names for the ⋯ menu ("Dec 31 · ACH AMEX EPAYMENT"): one Ignore this side each. */
        sides: readonly string[];
        onLink: () => void;
        onReject: () => void;
        onSkip: () => void;
        onRevert: () => void;
        onIgnore: (side: 0 | 1) => void;
    } = $props();

    const saveKeys = formatKeys("mod+Enter")
        .map((token) => token.text)
        .join("");
    let menuOpen = $state(false);
</script>

<footer
    class="flex h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-base-content/10 px-5 sm:px-8"
    data-testid="match-footer"
    data-state={dirty ? "dirty" : "clean"}
>
    {#key dirty}
        <div class="flex w-full items-center justify-between gap-3" in:footerFade={{reduced}}>
            <span class="flex items-center gap-2">
                <button type="button" class="btn gap-2 btn-success" onclick={onLink} disabled={busy} data-testid="match-link">
                    <ReviewIcon name="check" />
                    {dirty ? "Link and Save" : "Link"}
                    <kbd class="kbd border-success-content/30 bg-success-content/10 kbd-xs text-success-content">{saveKeys}</kbd>
                </button>
                {#if dirty}
                    <button type="button" class="btn gap-1 btn-ghost btn-sm" onclick={onRevert} data-testid="match-revert">
                        <ReviewIcon name="undo" class="h-3.5 w-3.5" /> Revert <kbd class="kbd kbd-xs">z</kbd>
                    </button>
                {/if}
            </span>
            <span class="flex items-center gap-1">
                <details
                    class="dropdown dropdown-end dropdown-top"
                    bind:open={menuOpen}
                    use:dismissible={{active: menuOpen, outside: true, onDismiss: () => (menuOpen = false)}}
                >
                    <summary class="btn btn-square btn-ghost btn-sm" aria-label="More actions" title="More actions" data-testid="match-menu">
                        <ReviewIcon name="ellipsis" />
                    </summary>
                    <ul class="menu dropdown-content z-30 mb-1 w-72 rounded-box border border-base-300 bg-base-200 p-2 text-sm shadow-xl">
                        {#each sides as label, side (side)}
                            <li>
                                <button
                                    type="button"
                                    disabled={busy}
                                    onclick={() => {
                                        menuOpen = false;
                                        onIgnore(side === 0 ? 0 : 1);
                                    }}
                                    data-testid="match-ignore-{side === 0 ? 'left' : 'right'}"
                                >
                                    <ReviewIcon name="ban" />
                                    <span class="min-w-0">Ignore this side <span class="block truncate text-xs text-base-content/55">{label}</span></span>
                                </button>
                            </li>
                        {/each}
                    </ul>
                </details>
                {#if canReject}
                    <button
                        type="button"
                        class="btn gap-2 btn-ghost"
                        onclick={onReject}
                        disabled={busy}
                        title={dirty ? "Remember these are not a pair, and save the descriptions" : "Remember these are not a pair"}
                        data-testid="match-reject"
                    >
                        <ReviewIcon name="x" /> Not a match <kbd class="kbd kbd-xs">x</kbd>
                    </button>
                {/if}
                <button type="button" class="btn gap-2 btn-ghost" onclick={onSkip} disabled={busy} data-testid="match-skip">
                    Skip <ReviewIcon name="arrow-right" />
                </button>
            </span>
        </div>
    {/key}
</footer>
