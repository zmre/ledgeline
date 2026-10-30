<!-- The dialog's fields for a free-standing comment (plan 31 C4): its text,
     without `; ` prefixes, and whether it is a section heading or a note. A
     new heading is written in Patrick's house style — 22 semicolons, then
     `;; Title` — and an existing one keeps its own framing. A `comment … end
     comment` block is edited verbatim and stays a block. -->
<script lang="ts">
    import type {ScenarioComment} from "../../../types";

    let {comment, onEdit}: {comment: ScenarioComment; onEdit: () => void} = $props();

    function setStyle(style: "note" | "section"): void {
        if (comment.style === style) return;
        comment.style = style;
        comment.edited = true;
        onEdit();
    }
</script>

<div class="flex flex-col gap-3" data-testid="comment-editor">
    {#if comment.style !== "block"}
        <div class="join" role="radiogroup" aria-label="Kind of comment">
            {#each [["section", "Section heading"], ["note", "Note"]] as const as [style, label] (style)}
                <button
                    type="button"
                    role="radio"
                    aria-checked={comment.style === style}
                    class={["btn join-item btn-sm", comment.style === style ? "btn-active btn-primary" : "btn-ghost"]}
                    onclick={() => setStyle(style)}>{label}</button
                >
            {/each}
        </div>
    {/if}
    <label class="flex flex-col gap-1">
        <span class="text-xs text-base-content/60">{comment.style === "section" ? "Title" : "Text"}</span>
        {#if comment.style === "section" && !comment.text.includes("\n")}
            <input
                type="text"
                class="input w-full input-sm"
                value={comment.text}
                placeholder="Operational expenses"
                aria-label="Section title"
                onchange={(e) => {
                    comment.text = e.currentTarget.value;
                    comment.edited = true;
                    onEdit();
                }}
            />
        {:else}
            <textarea
                class="textarea w-full font-mono text-sm"
                rows={Math.max(3, comment.text.split("\n").length + 1)}
                value={comment.text}
                placeholder="Assumes no price rises before 2027."
                aria-label={comment.style === "block" ? "Comment block text" : "Note text"}
                spellcheck="true"
                onchange={(e) => {
                    comment.text = e.currentTarget.value;
                    comment.edited = true;
                    onEdit();
                }}></textarea>
        {/if}
    </label>
</div>
