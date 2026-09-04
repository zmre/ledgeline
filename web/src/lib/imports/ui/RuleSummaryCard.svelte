<script lang="ts">
    // One rule, collapsed to the line a user scans with.
    //
    // The rules list used to render every rule as a full editor, all at once,
    // which made a file of any size a very long column of identical-looking
    // controls: finding the rule that sends AMAZON somewhere meant reading every
    // field of every rule on the way down. So the list shows this instead, and
    // opens `IfBlockCard` for the one rule being edited.
    //
    // The summary comes from `describeIfBlock`, which is a pure function with its
    // own tests — this component only decides how the line is framed. It is
    // deliberately ONE line, clipped with `truncate` rather than wrapped: a card
    // that grows to fit its rule is the thing collapsing it was meant to stop.
    // The whole line is also the `title`, so the clipped tail is a hover away.
    //
    // Reordering lives here as well as in the editor, because "later matches
    // win" is a property of the LIST and moving a rule is something you do while
    // reading it. Deleting does not: it is destructive and irreversible from
    // here, so it stays inside the opened editor where the rule can be read
    // first. `KeptItemCard` is the same card for an item that has no editor at
    // all — this one is not dimmed, because this rule can be opened.
    let {
        summary,
        position,
        total,
        disabled,
        onMoveUp,
        onMoveDown,
        onOpen,
    }: {
        summary: string;
        /** 1-based position in the rules list, which is what the user is reordering. */
        position: number;
        total: number;
        disabled: boolean;
        onMoveUp: () => void;
        onMoveDown: () => void;
        onOpen: () => void;
    } = $props();
</script>

<div class="card min-w-0 border border-base-content/10 bg-base-200" data-testid="imports-rule">
    <div class="flex min-w-0 items-center gap-2 p-2">
        <!-- Disabled at the bounds rather than hidden, and labelled exactly as
             the editor's own buttons are: which card is on screen must not
             change what a control is called. -->
        <div class="join">
            <button
                type="button"
                class="btn join-item btn-xs"
                disabled={disabled || position === 1}
                onclick={onMoveUp}
                aria-label="Move rule {position} up"
                title="Move up"
            >
                ↑
            </button>
            <button
                type="button"
                class="btn join-item btn-xs"
                disabled={disabled || position === total}
                onclick={onMoveDown}
                aria-label="Move rule {position} down"
                title="Move down"
            >
                ↓
            </button>
        </div>
        <span class="shrink-0 text-xs text-base-content/60">Rule {position}</span>
        <!-- The line itself is the control: a separate "edit" button beside a
             row of text is one more thing to aim at, and the text is the thing
             the user is already looking at.

             `shrink` is load-bearing, not decoration: daisyUI's `.btn` sets
             `flex-shrink: 0`, so without it the button sizes to the whole
             summary, `truncate` never fires, and one long rule puts the page
             into a horizontal scroll at 375px. Measured in a browser at that
             width — the button was 1034px wide inside a 343px card. -->
        <button
            type="button"
            class="btn min-w-0 shrink grow justify-start btn-ghost px-2 font-normal btn-sm"
            onclick={onOpen}
            aria-expanded="false"
            aria-label="Edit rule {position}"
            title={summary}
        >
            <span class="w-full truncate text-left text-sm">{summary}</span>
        </button>
    </div>
</div>
