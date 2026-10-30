<!-- One entry's card: the frame every kind of rule shares (plan 31 C3).

     The grip (the mouse's way to move it), the kind badges, the header line,
     the body, and on the right the Edit button and the `⋯` menu. A turned-off
     entry is dimmed with its header struck and an `off` badge that a screen
     reader hears as "turned off". The keyboard cursor is a ring, as in the
     import Rules list.

     The grip is `aria-hidden` and not focusable: the menu's Move up / Move
     down and J/K are the accessible path, and a second half-working keyboard
     control on the grip would only be noise (as the old table decided). -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import type {Entry} from "../../entries";
    import RowMenu from "../RowMenu.svelte";
    import KindBadge from "./KindBadge.svelte";

    interface MenuItem {
        label: string;
        onSelect: () => void;
        danger?: boolean;
    }

    let {
        entry,
        cursor,
        canDrag,
        outlined = false,
        hint = null,
        menu,
        onEdit = null,
        onActivate,
        onDragStart,
        onDragEnd,
        onHover,
        header,
        body,
    }: {
        entry: Entry;
        /** The keyboard cursor is on this entry. */
        cursor: boolean;
        /** Reordering is possible (no filter on, not read-only text). */
        canDrag: boolean;
        /** A sibling step of the hovered entry. */
        outlined?: boolean;
        /** A note under the header while the dialog works on this entry ("will be turned off"). */
        hint?: string | null;
        menu: MenuItem[];
        onEdit?: (() => void) | null;
        /** A click on the card: the cursor moves here. */
        onActivate: () => void;
        onDragStart: (event: DragEvent) => void;
        onDragEnd: () => void;
        onHover?: (inside: boolean) => void;
        /** The header line's text, after the badges. */
        header: Snippet<[string]>;
        body?: Snippet;
    } = $props();

    const headId = $props.id();
</script>

<!-- `onclick` here only moves the cursor; every action inside is a real button. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article
    class={[
        "group/entry relative scroll-mt-16 rounded-box border bg-base-200/55 py-1.5 pr-1.5 pl-1 transition-[border-color,background-color,opacity] duration-150",
        cursor
            ? "border-primary/70 ring-2 ring-primary/70"
            : outlined
              ? "border-dashed border-primary/45"
              : "border-base-content/8 hover:border-base-content/20",
        entry.disabled && "bg-base-200/25",
    ]}
    aria-labelledby={headId}
    aria-current={cursor ? "true" : undefined}
    data-testid="projection-entry"
    data-kind={entry.kind}
    data-key={entry.key}
    data-disabled={entry.disabled ? "true" : "false"}
    onclick={(event) => {
        // A control that moves the cursor ELSEWHERE (the step chip) says so;
        // its click must not then land the cursor back on this card.
        if (!(event.target instanceof Element && event.target.closest("[data-no-activate]") !== null)) onActivate();
    }}
    ondblclick={() => onEdit?.()}
    onpointerenter={() => onHover?.(true)}
    onpointerleave={() => onHover?.(false)}
>
    <div class="flex items-start gap-1">
        <span
            class={[
                "mt-0.5 w-4 shrink-0 text-center leading-5 select-none",
                canDrag ? "cursor-grab text-base-content/25 group-hover/entry:text-base-content/60" : "text-transparent",
            ]}
            draggable={canDrag ? "true" : "false"}
            aria-hidden="true"
            title={canDrag ? "Drag to move" : undefined}
            data-testid="drag-handle"
            ondragstart={onDragStart}
            ondragend={onDragEnd}>⠿</span
        >
        <div class={["min-w-0 grow", entry.disabled && "opacity-55"]}>
            <div class="flex min-h-5 flex-wrap items-center gap-x-2 gap-y-1">
                {#if entry.disabled}
                    <KindBadge kind="off" />
                {/if}
                {#each entry.badges as badge (badge)}
                    <KindBadge kind={badge} />
                {/each}
                {@render header(headId)}
            </div>
            {#if hint !== null}
                <p class="mt-1 text-xs text-warning" data-testid="entry-hint">{hint}</p>
            {/if}
            {#if body}
                {@render body()}
            {/if}
        </div>
        <div class="flex shrink-0 items-center gap-0.5 opacity-45 transition-opacity group-focus-within/entry:opacity-100 group-hover/entry:opacity-100">
            {#if onEdit !== null}
                <button
                    type="button"
                    class="btn h-6 min-h-6 btn-ghost px-2 font-normal btn-xs"
                    aria-label="Edit {entry.name}"
                    data-testid="entry-edit"
                    onclick={() => onEdit?.()}>Edit</button
                >
            {/if}
            <RowMenu label="Actions for {entry.name}" items={menu} />
        </div>
    </div>
</article>
