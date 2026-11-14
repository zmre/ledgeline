<script lang="ts">
    // A narrow journal card that answers a finger (plan 42 phase 3): swipe left
    // reveals the trailing actions, swipe right runs the leading one, and a
    // long press opens the row's menu. The thresholds are `swipe.ts`'s; this
    // only feeds them pointer events.
    //
    // Touch and pen only: a mouse has the card's tap and the desktop table.
    // `touch-action: pan-y` hands vertical drags to the browser before any
    // script sees them, so the page scrolls exactly as it did; a drag the
    // browser takes for a scroll arrives here as `pointercancel` and the card
    // stays put. A tap that was not a swipe or a long press reaches the card
    // untouched; one that ENDS a swipe or a long press is swallowed, so letting
    // go does not also open the transaction.
    import {untrack, type Snippet} from "svelte";
    import type {ClassValue} from "svelte/elements";
    import {haptic} from "#lib/native/share.js";
    import {rootPx} from "#lib/theme/scale.js";
    import {
        crossedCommit,
        decideAxis,
        dragOffset,
        LONG_PRESS_MS,
        restingOffset,
        scaledSwipe,
        settle,
        SWIPE_DEFAULTS,
        type SwipeAction,
        type SwipeAxis,
    } from "./swipe";
    import {swipeOpen} from "./swipeOpen.svelte";

    let {
        id,
        enabled,
        leading = null,
        trailing = [],
        onLongPress,
        class: className = "",
        children,
    }: {
        /** The card's transaction index: which card `swipeOpen` names. */
        id: number;
        /** Read-only journals swipe nothing. */
        enabled: boolean;
        /** Swipe right. */
        leading?: SwipeAction | null;
        /** Revealed by a swipe left, left to right. */
        trailing?: SwipeAction[];
        onLongPress?: () => void;
        class?: ClassValue;
        children: Snippet;
    } = $props();

    /** Each trailing button's width at 100 %, px. */
    const ACTION_PX = 72;
    const config = $derived(scaledSwipe(rootPx() / 16, {...SWIPE_DEFAULTS, actionsWidth: ACTION_PX * Math.max(1, trailing.length)}));

    let offset = $state(0);
    let dragging = $state(false);
    const isOpen = $derived(swipeOpen.current === id);

    // Another card opened (or this one was closed from outside): spring back.
    $effect(() => {
        if (isOpen) return;
        untrack(() => {
            if (!dragging && offset < 0) offset = 0;
        });
    });

    let start: {x: number; y: number; pointer: number} | null = null;
    let axis: SwipeAxis = "undecided";
    let base = 0;
    let last = {x: 0, t: 0};
    let velocity = 0;
    let pressTimer: ReturnType<typeof setTimeout> | undefined;
    let swallowClick = false;
    /** Fingers down on the card (plain, not reactive: only the handlers read it). */
    let pointers: number[] = [];

    const TONE: Record<SwipeAction["tone"], string> = {
        success: "bg-success text-success-content",
        info: "bg-info text-info-content",
        error: "bg-error text-error-content",
        neutral: "bg-neutral text-neutral-content",
    };

    function clearPress(): void {
        if (pressTimer !== undefined) clearTimeout(pressTimer);
        pressTimer = undefined;
    }

    function onpointerdown(event: PointerEvent): void {
        if (!enabled || event.pointerType === "mouse") return;
        pointers = [...pointers.filter((id) => id !== event.pointerId), event.pointerId];
        swallowClick = false;
        if (pointers.length > 1) {
            // A second finger: a pinch, not a swipe.
            finish(true);
            return;
        }
        start = {x: event.clientX, y: event.clientY, pointer: event.pointerId};
        axis = "undecided";
        base = isOpen ? -config.actionsWidth : 0;
        velocity = 0;
        last = {x: event.clientX, t: event.timeStamp};
        if (onLongPress !== undefined) {
            pressTimer = setTimeout(() => {
                pressTimer = undefined;
                if (start === null || axis !== "undecided") return;
                start = null;
                swallowClick = true;
                haptic("light");
                onLongPress?.();
            }, LONG_PRESS_MS);
        }
    }

    function onpointermove(event: PointerEvent): void {
        if (start === null || event.pointerId !== start.pointer) return;
        const dx = event.clientX - start.x;
        const dy = event.clientY - start.y;
        if (axis === "undecided") {
            axis = decideAxis(dx, dy, config);
            if (axis === "undecided") return;
            clearPress();
            if (axis === "vertical") {
                start = null;
                return;
            }
            dragging = true;
            (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
        }
        let next = dragOffset(base, dx, config);
        if (leading === null) next = Math.min(next, 0);
        if (leading !== null && crossedCommit(offset, next, config)) haptic("light");
        const dt = Math.max(1, event.timeStamp - last.t);
        velocity = (event.clientX - last.x) / dt;
        last = {x: event.clientX, t: event.timeStamp};
        offset = next;
    }

    function finish(cancelled: boolean): void {
        clearPress();
        start = null;
        if (!dragging) return;
        dragging = false;
        swallowClick = true;
        const end = cancelled ? (base < 0 ? "open" : "close") : settle(offset, velocity, config);
        if (end === "commit" && leading !== null) {
            haptic("success");
            leading.run();
        }
        const resting = end === "commit" ? "close" : end;
        offset = restingOffset(resting, config);
        if (resting === "open") swipeOpen.set(id);
        else if (isOpen) swipeOpen.set(null);
    }

    function onpointerup(event: PointerEvent): void {
        pointers = pointers.filter((id) => id !== event.pointerId);
        finish(false);
    }

    function onpointercancel(event: PointerEvent): void {
        pointers = pointers.filter((id) => id !== event.pointerId);
        finish(true);
    }

    /** Capture phase, so it runs before the card's own click. */
    function onclickcapture(event: MouseEvent): void {
        if (swallowClick || isOpen) {
            swallowClick = false;
            event.stopPropagation();
            event.preventDefault();
            if (isOpen) swipeOpen.set(null);
        }
    }

    function runTrailing(action: SwipeAction): void {
        swipeOpen.set(null);
        offset = 0;
        action.run();
    }
</script>

<div class={["relative overflow-hidden rounded-box", className]} data-testid="swipe-card">
    {#if enabled}
        {#if leading !== null && offset > 0}
            <div class={["absolute inset-0 flex items-center pl-4 text-sm font-medium", TONE[leading.tone]]} aria-hidden="true">
                <span class={["transition-transform", offset >= config.commitAt && "scale-110"]}>{leading.label}</span>
            </div>
        {/if}
        <!-- Only while the card is off to the left: at rest the actions would
             show through the card's anti-aliased rounded corners. -->
        {#if offset < 0 || isOpen}
            <div class="absolute inset-y-0 right-0 flex" style:width="{config.actionsWidth}px" inert={!isOpen} aria-hidden={!isOpen}>
                {#each trailing as action (action.label)}
                    <button
                        type="button"
                        class={["flex-1 text-sm font-medium", TONE[action.tone]]}
                        onclick={() => runTrailing(action)}
                        data-testid={action.testid}>{action.label}</button
                    >
                {/each}
            </div>
        {/if}
    {/if}
    <!-- `presentation`: the gestures are shortcuts; the card inside keeps its own button and tap. -->
    <div
        role="presentation"
        class={["relative h-full", enabled && "touch-pan-y select-none [-webkit-touch-callout:none]"]}
        style:transform={offset === 0 ? undefined : `translateX(${offset}px)`}
        style:transition={dragging ? "none" : "transform 180ms ease-out"}
        {onpointerdown}
        {onpointermove}
        {onpointerup}
        {onpointercancel}
        {onclickcapture}
    >
        {@render children()}
    </div>
</div>
