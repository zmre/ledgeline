<!-- A small (i) help button with a tooltip: an optional bold title line, a
     short paragraph, and an optional formula line in monospace.

     It opens on hover, on focus, and on tap or click, which keeps it open
     until the button loses focus. Escape hides it, even under the pointer,
     until the next hover, focus or tap. The content is a real element,
     referenced by `aria-describedby`, so a screen reader announces it when the
     button is focused; `data-tip` text, which this replaced, lived in CSS and
     was never announced. It stays in the DOM while closed (just `hidden`), so
     the description resolves either way.

     # Why the tooltip is fixed and portalled, not a daisyUI `.tooltip`

     It was a daisyUI `.tooltip`, whose content is ABSOLUTELY positioned inside
     the icon's wrapper at `z-index: 2`. That is only as high as the stacking
     context the icon sits in, and only as visible as its nearest clipping
     ancestor allows: a "More metrics" row's tooltip was cut off by the
     collapse panel's `overflow: hidden` and drawn under the "Biggest Expense
     Changes" card below it. So, like `RowMenu` and `AccountInput`'s popup, it
     is `position: fixed`, moved to <body> by `use:portal`, and placed from the
     trigger's viewport rect by the pure, unit-tested `tipPosition` — which
     also flips it to stay on screen. It follows the trigger on scroll and
     resize while open. `z-[1002]` clears daisyUI's `.modal` (999) and the
     app's own popups (1001), since an (i) can sit inside either.

     # Alignment

     The wrapper and the button are `inline-flex` around a `block` svg, so the
     whole thing is exactly the icon's 14px tall. An inline svg instead sat on
     the line box of the inherited font, and `align-middle` put its centre at
     baseline + half an x-height — about 3px below the centre of the label
     beside it in every call site. Each call site centres label and icon with
     `items-center`; this keeps the icon from carrying its own baseline. -->
<script lang="ts">
    import {portal, tipPosition, type TipPlacement} from "#lib/ui/anchoredPopup.js";

    let {
        label,
        text,
        title,
        formula,
        placement = "right",
    }: {
        /** The thing explained; the button is labelled "About <label>". */
        label: string;
        text: string;
        title?: string;
        formula?: string;
        /**
         * Where the tooltip prefers to open. `bottom-start` grows rightward
         * from the icon and `bottom-end` leftward, for icons near a card's left
         * or right edge. It flips if that side has no room.
         */
        placement?: TipPlacement;
    } = $props();

    const id = $props.id();
    let hovered = $state(false);
    let focused = $state(false);
    let pinned = $state(false);
    let dismissed = $state(false);
    const open = $derived(!dismissed && (hovered || focused || pinned));

    let trigger: HTMLButtonElement | undefined = $state();
    let tip: HTMLSpanElement | undefined = $state();
    let position = $state({top: 0, left: 0});

    function reposition(): void {
        if (trigger === undefined || tip === undefined) return;
        const viewport = {width: document.documentElement.clientWidth, height: document.documentElement.clientHeight};
        const size = tip.getBoundingClientRect();
        const {top, left} = tipPosition(trigger.getBoundingClientRect(), viewport, {width: size.width, height: size.height}, placement);
        position = {top, left};
    }

    // Runs after the DOM update that un-hides the tooltip and before paint, so
    // it is measured at its real size and never seen at the old position.
    // Scroll is captured on `document` because any ancestor may be the
    // scroller; the dashboard's cards scroll with the page, not themselves.
    $effect(() => {
        if (!open) return;
        reposition();
        window.addEventListener("resize", reposition);
        document.addEventListener("scroll", reposition, true);
        return () => {
            window.removeEventListener("resize", reposition);
            document.removeEventListener("scroll", reposition, true);
        };
    });

    function onkeydown(event: KeyboardEvent): void {
        if (event.key === "Escape") dismissed = true;
    }
</script>

<span class="inline-flex shrink-0 items-center" data-state={open ? "open" : "closed"} data-testid="info-tip">
    <button
        bind:this={trigger}
        type="button"
        class="inline-flex cursor-help items-center text-base-content/50 hover:text-base-content"
        aria-label="About {label}"
        aria-describedby="{id}-tip"
        onmouseenter={() => {
            hovered = true;
            dismissed = false;
        }}
        onmouseleave={() => (hovered = false)}
        onfocus={() => {
            focused = true;
            dismissed = false;
        }}
        onblur={() => {
            focused = false;
            pinned = false;
        }}
        onclick={() => {
            pinned = true;
            dismissed = false;
        }}
        {onkeydown}
    >
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="block h-3.5 w-3.5" aria-hidden="true">
            <path
                fill-rule="evenodd"
                d="M8 15A7 7 0 1 0 8 1a7 7 0 0 0 0 14Zm.75-10.25a.75.75 0 1 1-1.5 0 .75.75 0 0 1 1.5 0ZM7 7a.75.75 0 0 0 0 1.5h.25v3a.75.75 0 0 0 1.5 0v-3.75A.75.75 0 0 0 8 7H7Z"
                clip-rule="evenodd"
            />
        </svg>
    </button>
    <!-- Inside the wrapper, so the wrapper stays this component's only root
         node and Svelte's unmount never anchors on a node that has moved. -->
    <span
        use:portal
        bind:this={tip}
        id="{id}-tip"
        role="tooltip"
        class="pointer-events-none fixed z-[1002] {open
            ? 'flex'
            : 'hidden'} w-max max-w-[min(16rem,calc(100vw-1rem))] flex-col gap-1 rounded-field bg-neutral px-2 py-1 text-left text-sm leading-tight font-normal whitespace-normal text-neutral-content shadow-lg"
        style="top: {position.top}px; left: {position.left}px"
    >
        {#if title !== undefined}<span class="font-semibold">{title}</span>{/if}
        <span>{text}</span>
        {#if formula !== undefined}<span class="font-mono text-xs opacity-80">{formula}</span>{/if}
    </span>
</span>
