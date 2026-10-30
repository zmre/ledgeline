<!-- A free-standing comment piece (plan 31 C4), in one of three styles:

     - **section** — a `;;;;` divider block, drawn as a section HEADING: small
       caps with a rule, sticky under the page header while its run scrolls
       (the stickiness is on the list item, see `ScenarioList`).
     - **note** — prose between entries, drawn as the comment it is.
     - **block** — hledger's `comment … end comment`, verbatim.

     Each is an entry like any other: it moves, is edited in the dialog and is
     deleted from its menu. A comment cannot be turned off; it already is. -->
<script lang="ts">
    import type {Entry} from "../../entries";
    import RowMenu from "../RowMenu.svelte";
    import type {FrameProps} from "./frame";

    let {entry, frame}: {entry: Extract<Entry, {kind: "comment"}>; frame: FrameProps} = $props();

    const comment = $derived(entry.comment);
    const lines = $derived(comment.text.split("\n"));
    const title = $derived(lines.find((line) => line.trim() !== "")?.trim() ?? "");
    const rest = $derived(
        lines
            .slice(lines.findIndex((line) => line.trim() !== "") + 1)
            .join("\n")
            .trim()
    );
    const headId = $props.id();
</script>

{#snippet actions()}
    <div class="flex shrink-0 items-center gap-0.5 opacity-45 transition-opacity group-focus-within/entry:opacity-100 group-hover/entry:opacity-100">
        {#if frame.onEdit !== null}
            <button
                type="button"
                class="btn h-6 min-h-6 btn-ghost px-2 font-normal btn-xs"
                aria-label="Edit {entry.name}"
                data-testid="entry-edit"
                onclick={() => frame.onEdit?.()}>Edit</button
            >
        {/if}
        <RowMenu label="Actions for {entry.name}" items={frame.menu} />
    </div>
{/snippet}

{#snippet grip()}
    <span
        class={[
            "w-4 shrink-0 text-center leading-5 select-none",
            frame.canDrag ? "cursor-grab text-base-content/25 group-hover/entry:text-base-content/60" : "text-transparent",
        ]}
        draggable={frame.canDrag ? "true" : "false"}
        aria-hidden="true"
        title={frame.canDrag ? "Drag to move" : undefined}
        data-testid="drag-handle"
        ondragstart={frame.onDragStart}
        ondragend={frame.onDragEnd}>⠿</span
    >
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article
    class={[
        "group/entry relative scroll-mt-16 rounded-box",
        comment.style === "section" ? "py-1 pr-1.5 pl-1" : "py-1 pr-1.5 pl-1",
        frame.cursor ? "ring-2 ring-primary/70" : "",
    ]}
    aria-labelledby={headId}
    aria-current={frame.cursor ? "true" : undefined}
    data-testid="projection-entry"
    data-kind="comment"
    data-style={comment.style}
    data-key={entry.key}
    data-disabled="false"
    onclick={frame.onActivate}
    ondblclick={() => frame.onEdit?.()}
>
    {#if comment.style === "section"}
        <div class="flex items-center gap-2">
            {@render grip()}
            <h3 id={headId} class="max-w-[60%] truncate text-xs font-semibold tracking-[0.14em] text-base-content/85 uppercase">
                <span class="sr-only">Section: </span>{title === "" ? "untitled section" : title}
            </h3>
            {#if rest !== ""}
                <span class="hidden min-w-0 truncate text-xs text-base-content/45 sm:inline">{rest.split("\n").join(" · ")}</span>
            {/if}
            <span class="h-px min-w-6 grow bg-linear-to-r from-base-content/25 via-base-content/12 to-transparent" aria-hidden="true"></span>
            {@render actions()}
        </div>
    {:else}
        <div class="flex items-start gap-1">
            <span class="mt-0.5">{@render grip()}</span>
            <div class="min-w-0 grow">
                <h3 id={headId} class="sr-only">{comment.style === "block" ? "Comment block" : "Note"}</h3>
                {#if comment.style === "block"}
                    <div class="rounded border border-dashed border-base-content/15 px-2 py-1 font-mono text-xs text-base-content/50">
                        <div class="text-base-content/35">comment</div>
                        <pre class="overflow-x-auto whitespace-pre-wrap">{comment.text}</pre>
                        <div class="text-base-content/35">end comment</div>
                    </div>
                {:else}
                    <p class="border-l-2 border-base-content/12 pl-2 font-mono text-xs leading-5 whitespace-pre-wrap text-base-content/55 italic">
                        {lines.map((line) => `; ${line}`).join("\n")}
                    </p>
                {/if}
            </div>
            {@render actions()}
        </div>
    {/if}
</article>
