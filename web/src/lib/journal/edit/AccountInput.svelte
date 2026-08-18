<script lang="ts">
    // Account text field with a real combobox over the journal's account names.
    //
    // This used to be a native `<datalist>`. That could not Tab-complete at all,
    // and its matching was whatever the browser engine felt like — which differs
    // between the WKWebView this app ships in and the browser `just dev` runs in.
    // The matching now lives in `accountMatch.ts` (segment-aware fuzzy, heavily
    // unit-tested) and the popup is ours, so both are the same everywhere.
    //
    // The name is unchanged because the role is: an account text field, used by
    // the popup's posting rows, the inline category editor, and three import
    // panels. Renaming would have touched five call sites for no behavioural gain.
    //
    // Escape closes the POPUP first and only then reaches the parent. That is the
    // fix for a real bug: in the transaction popup this component was passed no
    // `onCancel`, so Escape did nothing locally and bubbled to the modal wrapper,
    // discarding the whole half-typed transaction.
    import {tick} from "svelte";
    import {TYPING_ATTRIBUTE} from "$lib/keys/target";
    import {popupPosition} from "$lib/ui/anchoredPopup";
    import {matchAccounts, tabCompletion} from "./accountMatch";

    let {
        value = $bindable(""),
        accountNames,
        placeholder = "account",
        autofocus = false,
        size = "sm",
        disabled = false,
        onCommit,
        onCancel,
    }: {
        value?: string;
        accountNames: string[];
        placeholder?: string;
        autofocus?: boolean;
        size?: "sm" | "xs";
        disabled?: boolean;
        onCommit?: () => void;
        onCancel?: () => void;
    } = $props();

    /** Rows to show at once before the list scrolls. */
    const VISIBLE_ROWS = 8;
    const ROW_HEIGHT = 28;

    let open = $state(false);
    let highlighted = $state(0);
    /**
     * What the user actually typed, as opposed to what Tab has since written
     * into the field.
     *
     * Cycling needs this. Once Tab writes a full account name, the field's value
     * IS that account — so matching on the field would find one candidate and
     * there would be nothing left to cycle through. Shell completion cycles over
     * the candidates for the stem you typed, and so does this.
     */
    let stem = $state("");
    let field = $state<HTMLInputElement | null>(null);
    let popup = $state<HTMLUListElement | null>(null);
    let placement = $state({top: 0, left: 0, width: 0, maxHeight: 0, below: true});
    // Guards the blur-vs-click race: a pointerdown on an option blurs the input,
    // and without this the blur would commit the OLD text and unmount the popup
    // before the click ever landed. Same shape as `commitDesc`'s `editingDesc`
    // guard in TransactionRow, for the same reason.
    let choosing = false;

    const matches = $derived(open ? matchAccounts(stem, accountNames) : []);
    // Stable across renders, unlike the `Math.random()` id the old <datalist>
    // used — which changed on every remount and could not be asserted in a test.
    const uid = $props.id();
    const listId = `acct-list-${uid}`;

    function reposition(): void {
        if (field === null) return;
        const rect = field.getBoundingClientRect();
        placement = popupPosition(rect, {width: window.innerWidth, height: window.innerHeight}, VISIBLE_ROWS * ROW_HEIGHT);
    }

    async function show(newStem: string = value): Promise<void> {
        if (disabled) return;
        stem = newStem;
        open = true;
        highlighted = 0;
        await tick();
        reposition();
    }

    function hide(): void {
        open = false;
        highlighted = 0;
    }

    function choose(name: string): void {
        value = name;
        stem = name;
        hide();
        field?.focus();
    }

    function move(delta: number): void {
        if (matches.length === 0) return;
        const next = highlighted + delta;
        highlighted = next < 0 ? matches.length - 1 : next >= matches.length ? 0 : next;
        // The popup scrolls internally, so the highlighted row has to be brought
        // into view by hand. It is always mounted (this list is not virtualized),
        // so `scrollIntoView` is honest here — unlike in the journal table.
        popup?.querySelector(`[data-at="${highlighted}"]`)?.scrollIntoView({block: "nearest"});
    }

    function onInput(event: Event): void {
        value = (event.currentTarget as HTMLInputElement).value;
        // Opens on TYPING, not on focus: tabbing through the transaction popup's
        // posting rows should not spray popups down the form.
        void show();
    }

    function onKeydown(event: KeyboardEvent): void {
        // Mid-composition every keystroke is provisional; committing or
        // navigating on one would eat a half-typed character.
        if (event.isComposing) return;

        if (event.key === "Escape") {
            // Claim Escape ONLY when this component actually consumes it —
            // `preventDefault` is how everything else in the app is told a key is
            // spoken for, so claiming one we ignored silently swallows it.
            //
            // Popup open  → close the popup (the fix for Escape discarding a
            //               half-typed transaction).
            // Popup shut  → cancel, if the caller wants one (the inline row
            //               editor does).
            // Neither     → not ours. Let it reach the enclosing modal, which is
            //               what makes a second Escape close the popup.
            if (open) {
                event.preventDefault();
                hide();
            } else if (onCancel !== undefined) {
                event.preventDefault();
                onCancel();
            }
            return;
        }

        if (event.key === "ArrowDown" || (event.ctrlKey && event.key === "n")) {
            event.preventDefault();
            if (open) move(1);
            else void show();
            return;
        }
        if (event.key === "ArrowUp" || (event.ctrlKey && event.key === "p")) {
            event.preventDefault();
            if (open) move(-1);
            return;
        }
        if (open && (event.key === "Home" || event.key === "End")) {
            event.preventDefault();
            highlighted = event.key === "Home" ? 0 : matches.length - 1;
            move(0);
            return;
        }

        if (event.key === "Enter") {
            if (open && matches.length > 0) {
                // Accept the completion. `preventDefault` also suppresses the
                // form's implicit submission, which is what makes "Enter accepts,
                // a second Enter saves" work with no `stopPropagation`.
                event.preventDefault();
                choose(matches[highlighted].name);
                return;
            }
            hide();
            onCommit?.();
            return;
        }

        if (event.key === "Tab") {
            // THE anti-trap rules. Shift+Tab is always ordinary focus traversal,
            // and forward Tab falls through whenever it has nothing to add — so
            // there is always a way out of this field. Hijacking Tab
            // unconditionally would make the transaction popup a keyboard trap.
            if (event.shiftKey) {
                hide();
                return;
            }
            const candidates = open ? matches : matchAccounts(value, accountNames);
            const completion = tabCompletion(open ? stem : value, candidates);
            if (completion !== null) {
                event.preventDefault();
                value = completion;
                // The completed prefix is now "typed", so it becomes the stem.
                void show(completion);
                return;
            }
            // Nothing left to complete: the user is choosing between equally-good
            // options, so cycle over them. Otherwise let Tab leave the field.
            if (open && matches.length > 1) {
                event.preventDefault();
                // Only advance if the field already shows the highlighted match.
                // Otherwise the first Tab would skip the first candidate — shell
                // completion lands on it before moving on.
                if (value === matches[highlighted].name) move(1);
                value = matches[highlighted].name;
                return;
            }
            hide();
        }
    }

    function onBlur(): void {
        if (choosing) return;
        hide();
        onCommit?.();
    }

    // Svelte's `autofocus` warning is fine here: focus follows an explicit click.
    function focusOnMount(node: HTMLInputElement): void {
        if (autofocus) node.focus();
    }
</script>

<!-- `data-keys-typing` tells the global keymap that this whole subtree owns the
     keyboard, so `j` does not scroll the journal while you are naming an account
     and the arrow keys in the popup stay ours. -->
<div class="relative" {...{[TYPING_ATTRIBUTE]: ""}}>
    <input
        bind:this={field}
        type="text"
        class="input {size === 'xs' ? 'input-xs' : 'input-sm'} w-full"
        bind:value
        {placeholder}
        {disabled}
        role="combobox"
        aria-label="Account"
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={open && matches.length > 0 ? `${listId}-${highlighted}` : undefined}
        autocomplete="off"
        spellcheck="false"
        oninput={onInput}
        onkeydown={onKeydown}
        onblur={onBlur}
        use:focusOnMount
    />
</div>

{#if open && matches.length > 0}
    <!-- Fixed and portalled to <body> (see anchoredPopup.ts): both of this
         component's homes are inside a container that clips on both axes.
         `z-[1001]` clears daisyUI's `.modal`, which is 999. -->
    <ul
        bind:this={popup}
        id={listId}
        role="listbox"
        aria-label="Account suggestions"
        class="menu bg-base-200 rounded-box border-base-300 fixed z-[1001] flex-nowrap overflow-y-auto border p-1 shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px; max-height: {placement.maxHeight}px"
        {...{[TYPING_ATTRIBUTE]: ""}}
    >
        {#each matches as match, at (match.name)}
            <li>
                <!-- A real <button>, so it is focusable, announced, and clickable
                     without any of the a11y warnings a clickable <li> would raise.
                     `onpointerdown` sets the guard BEFORE the input's blur fires. -->
                <button
                    type="button"
                    id="{listId}-{at}"
                    data-at={at}
                    role="option"
                    aria-selected={at === highlighted}
                    class="block w-full truncate px-2 py-1 text-left text-sm {at === highlighted ? 'bg-primary/25' : ''}"
                    tabindex="-1"
                    onpointerdown={() => (choosing = true)}
                    onclick={() => {
                        choosing = false;
                        choose(match.name);
                    }}
                >
                    {match.name}
                </button>
            </li>
        {/each}
    </ul>
{/if}

<svelte:window onresize={() => open && reposition()} onscroll={() => open && reposition()} />
