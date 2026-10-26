<!-- A row's DESCRIPTION: an icon whose tooltip shows it, and whose click edits it.

     The description is the `~` rule's header text — `unbudgeted — average over
     2025-11-01 to 2026-10-22` on a seeded row, whatever the user wrote on their
     own. It used to be reachable only as the "estimated" badge's `title`, so a
     row whose number had moved on still claimed to be an average, and nobody
     could fix it.

     # One description per RULE

     A `~` block has one header. Rows that are postings of one rule share it,
     so editing one edits them all, and both the tooltip and the editor say so
     ("shared by 3 lines of this rule") rather than letting a user believe they
     changed only the row they clicked.

     # Hover AND focus

     A `title` attribute shows on hover only, never on keyboard focus, and not
     at all on touch. So the tooltip is a real element: shown on pointer-enter
     and on focus, hidden on leave and blur, and PORTALLED for the reason
     `RowMenu` is — every table here sits in `overflow-x-auto`, which clips on
     both axes. Its text is also the button's `aria-describedby`, through a
     visually hidden copy that is always in the document (a description that
     only exists while hovered describes nothing to a screen reader).

     # The editor

     A small portalled popover with one text input. Enter saves, Escape cancels
     (through `keys/dismissible`, which also hands focus back to the icon), and
     a click outside cancels. What the file writer would refuse — a `;`, a line
     break, more than 256 bytes — is said under the box and keeps Save disabled,
     rather than failing the save later. -->
<script lang="ts">
    import {dismissible} from "#lib/keys/dismissible.js";
    import {menuPosition, portal} from "#lib/ui/anchoredPopup.js";
    import {noteProblem} from "../scenarioModel";

    let {
        note,
        label,
        shared = 1,
        stale = false,
        onSave,
    }: {
        /** The rule's description as it stands. */
        note: string;
        /** The row's name — unique on the page, as every row control's is. */
        label: string;
        /** How many lines share this rule, and so this description. */
        shared?: number;
        /** The seed's automatic text, under an amount that has since changed. */
        stale?: boolean;
        /** Called with the new text on Save. Never called on Cancel. */
        onSave: (next: string) => void;
    } = $props();

    const uid = $props.id();
    const describedBy = `${uid}-note`;

    /** `w-72`. A number because the style is inline. */
    const WIDTH = 288;
    const EDITOR_HEIGHT = 150;
    const TIP_HEIGHT = 80;

    let trigger = $state<HTMLButtonElement | null>(null);
    let editor = $state<HTMLDivElement | null>(null);
    let tipOpen = $state(false);
    let editing = $state(false);
    let draft = $state("");
    let placement = $state({top: 0, left: 0, width: WIDTH, maxHeight: 0, below: true});

    const problem = $derived(noteProblem(draft));

    /** The words the tooltip — and a screen reader — gets. */
    const description = $derived.by(() => {
        const parts = [note.trim() === "" ? "No description. Click to add one." : note];
        if (shared > 1) parts.push(`Shared by the ${shared} lines of this rule: editing it changes all of them.`);
        if (stale) parts.push("The amount has changed since this estimate was made, so the description may be out of date.");
        return parts.join(" ");
    });

    /** The viewport margin `anchoredPopup` keeps. */
    const MARGIN = 8;

    /**
     * Below (or above) the icon, LEFT-aligned to it. `menuPosition` aligns a
     * menu's right edge to its trigger, which suits a `⋯` in the last column
     * and here would lay the popup back over the account box it describes.
     */
    function place(height: number): void {
        if (trigger === null) return;
        const rect = trigger.getBoundingClientRect();
        const below = menuPosition(rect, {width: window.innerWidth, height: window.innerHeight}, {width: WIDTH, height});
        placement = {...below, left: Math.max(MARGIN, Math.min(rect.left, window.innerWidth - below.width - MARGIN))};
    }

    function showTip(): void {
        if (editing) return;
        place(TIP_HEIGHT);
        tipOpen = true;
    }

    function hideTip(): void {
        tipOpen = false;
    }

    function open(): void {
        if (editing) {
            cancel();
            return;
        }
        // Focused BEFORE the editor mounts, so `dismissible` records this icon
        // as the opener and hands focus back to it — macOS WebKit does not
        // focus a button on click by itself (see `RowMenu`).
        trigger?.focus();
        tipOpen = false;
        draft = note;
        place(EDITOR_HEIGHT);
        editing = true;
    }

    function cancel(): void {
        editing = false;
    }

    function save(): void {
        if (problem !== null) return;
        editing = false;
        const next = draft.trim();
        if (next !== note) onSave(next);
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== "Enter") return;
        event.preventDefault();
        save();
    }

    // Outside click cancels, counting the icon as inside — otherwise clicking
    // it to close would dismiss on pointerdown and re-open on the click behind
    // it. Scroll cancels too: the editor is anchored to a row inside a scroller
    // a window listener never sees.
    $effect(() => {
        if (!editing) return;
        const onPointerDown = (event: PointerEvent): void => {
            const target = event.target as Node | null;
            if (target === null) return;
            if (trigger?.contains(target) === true || editor?.contains(target) === true) return;
            cancel();
        };
        document.addEventListener("pointerdown", onPointerDown, true);
        return () => document.removeEventListener("pointerdown", onPointerDown, true);
    });
</script>

<button
    bind:this={trigger}
    type="button"
    class="btn relative btn-ghost px-1 btn-xs {note.trim() === '' ? 'text-base-content/30' : 'text-base-content/70'}"
    aria-label="Description for {label}"
    aria-describedby={describedBy}
    aria-haspopup="dialog"
    aria-expanded={editing}
    data-testid="note-button"
    data-stale={stale ? "true" : undefined}
    onclick={open}
    onpointerenter={showTip}
    onpointerleave={hideTip}
    onfocus={showTip}
    onblur={hideTip}
>
    <!-- A speech bubble: "there is something said about this row". -->
    <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2z" stroke-linejoin="round" />
        {#if note.trim() !== ""}
            <path d="M5 6h6M5 8h4" stroke-linecap="round" />
        {/if}
    </svg>
    {#if stale}
        <!-- The seeded text under a changed amount. A dot, not a word: it is a
             hint to look, and the tooltip says what it means. -->
        <span class="absolute top-0.5 right-0.5 h-1.5 w-1.5 rounded-full bg-warning" data-testid="note-stale"></span>
    {/if}
</button>
<span id={describedBy} class="sr-only">{description}</span>

{#if tipOpen && !editing}
    <div
        use:portal
        role="tooltip"
        aria-hidden="true"
        data-testid="note-tooltip"
        class="pointer-events-none fixed z-[1001] rounded-box border border-base-300 bg-base-200 px-2 py-1 text-xs shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px"
    >
        {#if note.trim() === ""}
            <span class="text-base-content/60">No description. Click to add one.</span>
        {:else}
            <span class="whitespace-pre-wrap">{note}</span>
        {/if}
        {#if shared > 1}
            <span class="mt-1 block text-base-content/60">Shared by the {shared} lines of this rule: editing it changes all of them.</span>
        {/if}
        {#if stale}
            <span class="mt-1 block text-warning">The amount has changed since this estimate was made, so the description may be out of date.</span>
        {/if}
    </div>
{/if}

{#if editing}
    <div
        use:portal
        use:dismissible={{onDismiss: cancel, trap: true}}
        bind:this={editor}
        role="dialog"
        aria-label="Edit the description for {label}"
        data-testid="note-editor"
        class="fixed z-[1001] flex flex-col gap-2 rounded-box border border-base-300 bg-base-200 p-2 text-xs shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px"
    >
        <input
            type="text"
            class="input w-full input-xs {problem === null ? '' : 'input-error'}"
            bind:value={draft}
            aria-label="Description for {label}"
            aria-invalid={problem !== null}
            placeholder="What this row is"
            onkeydown={onKeydown}
        />
        {#if problem !== null}
            <span class="text-error" role="alert">{problem}</span>
        {:else if shared > 1}
            <span class="text-base-content/60">Shared by the {shared} lines of this rule: this changes all of them.</span>
        {/if}
        {#if stale}
            <span class="text-warning">The amount has changed since this estimate was made.</span>
        {/if}
        <div class="flex justify-end gap-1">
            <button type="button" class="btn btn-ghost btn-xs" onclick={cancel}>Cancel</button>
            <button type="button" class="btn btn-primary btn-xs" disabled={problem !== null} onclick={save}>Save</button>
        </div>
    </div>
{/if}
