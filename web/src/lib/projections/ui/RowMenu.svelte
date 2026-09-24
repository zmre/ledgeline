<!-- One row's `⋯` menu: a trigger button, and a menu PORTALLED to <body>.

     # Why this is not a `<details class="dropdown">`

     It was, and it was invisible. A daisyUI dropdown positions its content
     ABSOLUTELY, inside whatever rendered it — and every table in the
     Projections tab is wrapped in `overflow-x-auto`. Per CSS spec a
     non-`visible` overflow on one axis computes the other to `auto`, so that
     wrapper clips on BOTH axes: the menu of a row near the bottom of a section
     opened downwards into clipped space and nothing appeared at all. This is
     the same failure, and the same fix, as `AccountInput`'s popup — see
     `anchoredPopup.ts` and plan 15 §"A regression this uncovered".

     So: `position: fixed`, moved to `<body>` by `use:portal`, and placed from
     the trigger's viewport rect by `menuPosition`. The arithmetic is pure and
     unit-tested, because jsdom has no layout engine and a component test could
     not tell a correct placement from a broken one (web/README.md).

     # Dismissal

     A native `<details>` does NOT close on Escape — that is `<dialog>`, and the
     claim that it did is why this menu could be opened and then not got rid of.
     Escape, focus restore and the topmost-only rule come from
     `keys/dismissible.ts`; outside-click and scroll are handled here, for the
     reasons given at the `$effect`.

     Items are real `<button>`s so a test can find them by role and name without
     asserting on geometry, and so the keyboard reaches them at all. -->
<script lang="ts">
    import {dismissible} from "$lib/keys/dismissible";
    import {menuPosition, portal} from "$lib/ui/anchoredPopup";

    interface Item {
        /** Also the `{#each}` key, so two items of one menu must not share one. */
        label: string;
        onSelect: () => void;
        /** Destructive. Rendered in the error colour. */
        danger?: boolean;
    }

    let {
        label,
        items,
    }: {
        /**
         * The trigger's accessible name.
         *
         * MUST be unique on the page. Every blank row used to be "a new row",
         * which left the one row a user most wants to delete — the half-typed
         * one — with no name to reach it by.
         */
        label: string;
        items: Item[];
    } = $props();

    /** The menu's width in px, which is `w-52`. A number because the style is inline. */
    const WIDTH = 208;
    /** Rough item height and padding. Used ONLY to decide whether to flip up. */
    const ITEM_HEIGHT = 36;
    const PADDING = 16;

    let open = $state(false);
    let trigger = $state<HTMLButtonElement | null>(null);
    let menu = $state<HTMLUListElement | null>(null);
    let placement = $state({top: 0, left: 0, width: WIDTH, maxHeight: 0, below: true});

    function reposition(): void {
        if (trigger === null) return;
        placement = menuPosition(
            trigger.getBoundingClientRect(),
            {width: window.innerWidth, height: window.innerHeight},
            {width: WIDTH, height: items.length * ITEM_HEIGHT + PADDING}
        );
    }

    function close(): void {
        open = false;
    }

    function toggle(): void {
        if (open) {
            // The trigger closes what it opened. Without this the only way out
            // of an open menu would be Escape or a click somewhere else, and
            // clicking `⋯` again — the obvious thing — would do nothing.
            close();
            return;
        }
        // Focused BEFORE the menu mounts, so `dismissible` captures this button
        // as the opener and hands focus back to it on dismissal. macOS WebKit
        // does not focus a button on click by itself (the engine quirk
        // `ColumnMenu` documents), so it is done explicitly.
        trigger?.focus();
        // Measured before mounting too: the placement depends only on the
        // trigger's rect and the item count, so there is nothing to await and
        // no frame in which the menu is painted at the last row's position.
        reposition();
        open = true;
    }

    function choose(item: Item): void {
        close();
        item.onSelect();
    }

    /** Roving focus, because a portalled menu is nowhere near its trigger in tab order. */
    function onMenuKeydown(event: KeyboardEvent): void {
        if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
        event.preventDefault();
        const buttons = [...(menu?.querySelectorAll("button") ?? [])];
        if (buttons.length === 0) return;
        const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const step = event.key === "ArrowDown" ? 1 : -1;
        buttons[(at + step + buttons.length) % buttons.length].focus();
    }

    // The two dismissals `dismissible` cannot do for us. Attached only while
    // open, so there is no idle document listener.
    //
    //   - OUTSIDE CLICK has to count the TRIGGER as inside. `dismissible`'s own
    //     `outside` knows only about the node it decorates, and a portalled
    //     menu's trigger is not inside that node — so clicking `⋯` to close
    //     would dismiss on pointerdown and the click right behind it would open
    //     the menu straight back up, leaving it stuck open for good.
    //   - SCROLL closes rather than repositions. This menu is anchored to a row
    //     inside `overflow-x-auto`, and `<svelte:window onscroll>` never sees
    //     that scroller; a capturing listener sees every scroller there is, and
    //     a menu that closes can never drift away from the row it belongs to.
    $effect(() => {
        if (!open) return;
        const onPointerDown = (event: PointerEvent): void => {
            const target = event.target as Node | null;
            if (target === null) return;
            if (trigger?.contains(target) === true || menu?.contains(target) === true) return;
            close();
        };
        document.addEventListener("pointerdown", onPointerDown, true);
        document.addEventListener("scroll", close, true);
        return () => {
            document.removeEventListener("pointerdown", onPointerDown, true);
            document.removeEventListener("scroll", close, true);
        };
    });
</script>

<button
    bind:this={trigger}
    type="button"
    class="btn btn-ghost btn-xs"
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label={label}
    data-testid="row-menu-trigger"
    onclick={toggle}
>
    ⋯
</button>

{#if open}
    <!-- `bg-base-200` and a border, NOT `bg-base-100`: the page itself is
         `bg-base-100` (`+layout.svelte`), so the old menu was the same colour as
         what it covered and a shadow was the only thing saying it was there.
         This is the surface `AccountInput`'s popup and `ColumnMenu` already use.
         `z-[1001]` clears daisyUI's `.modal`, which is 999. -->
    <ul
        use:portal
        use:dismissible={{onDismiss: close, trap: true}}
        bind:this={menu}
        role="menu"
        aria-label={label}
        data-testid="row-menu"
        class="menu fixed z-[1001] flex-nowrap overflow-y-auto rounded-box border border-base-300 bg-base-200 p-2 shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {placement.width}px; max-height: {placement.maxHeight}px"
        onkeydown={onMenuKeydown}
    >
        {#each items as item (item.label)}
            <li>
                <button type="button" role="menuitem" class={item.danger === true ? "text-error" : ""} onclick={() => choose(item)}>
                    {item.label}
                </button>
            </li>
        {/each}
    </ul>
{/if}

<svelte:window onresize={() => open && reposition()} />
