<script lang="ts">
    // A list of account renames, `from` above `to`, one pair per block.
    //
    // # Why not side by side
    //
    // These strings are long and neither end may be truncated: they are account
    // names, and the tail is where the difference between two of them usually
    // is. `PW Roth IRA - 3077:cash → assets:morganstanley:pw-roth-ira:cash` is
    // an ordinary one and it is 62 characters. Two columns give each end half
    // the width, which at the shell's 375px mobile case is about twenty
    // characters before both sides start wrapping — so the "columns" become two
    // ragged paragraphs beside each other, which is the layout that was reported
    // as unreadable in the first place.
    //
    // Stacked, each end gets the WHOLE width and wraps at most once. The arrow
    // moves to the front of the second line and sits in a fixed 1rem gutter, so
    // a wrapped `to` hangs under itself instead of under the arrow, and the eye
    // has one vertical line to follow down the list. There is no breakpoint,
    // because there is no width at which the other layout is better.
    //
    // # The pair is one sentence to a screen reader
    //
    // The visible halves are `aria-hidden` and `renameText` supplies the whole
    // pair to assistive technology as one string. Reading out "PW Roth IRA -
    // 3077:cash" and "assets:morganstanley:pw-roth-ira:cash" as two unrelated
    // fragments — with a `→` that most screen readers pronounce as nothing at
    // all — loses the only thing the list is saying.
    import {renameText} from "../aliasModel";
    import type {AliasRename} from "../importTypes";

    let {
        renames,
        testid,
    }: {
        renames: readonly AliasRename[];
        /** `data-testid` for the list itself; the pairs are addressed through it. */
        testid?: string;
    } = $props();
</script>

<ul class="flex flex-col gap-2" data-testid={testid}>
    {#each renames as rename (`${rename.from}→${rename.to}`)}
        <li class="grid grid-cols-[1rem_minmax(0,1fr)] items-baseline gap-x-1 leading-snug">
            <span class="sr-only col-span-2">{renameText(rename)}</span>
            <span class="col-span-2 font-mono text-xs break-all" aria-hidden="true">{rename.from}</span>
            <!-- `opacity-60`, not a `text-base-content/50`: this list renders
                 inside alerts that set their own foreground colour, and a
                 base-content arrow on a warning alert is the one thing on it
                 that does not match. -->
            <span class="text-xs opacity-60 select-none" aria-hidden="true">→</span>
            <span class="font-mono text-xs break-all" aria-hidden="true">{rename.to}</span>
        </li>
    {/each}
</ul>
