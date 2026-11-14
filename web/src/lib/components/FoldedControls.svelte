<script lang="ts">
    // A page's control bar that folds behind a summary on a narrow screen
    // (plan 42). Below `sm` the controls are a phone's whole first screen
    // before any figures show, so one button says what they are set to
    // ("Showing Year to date · depth 3") and opens them; from `sm` up the bar
    // is always shown and the button is not. Budget, Reports, Projections and
    // Holdings share it.
    //
    // The panel is a flex row; `class` styles it (gaps, padding, background).
    import type {Snippet} from "svelte";
    import type {ClassValue} from "svelte/elements";

    let {
        id,
        summary,
        lead = "Showing",
        testid,
        open = $bindable(false),
        class: className = "",
        children,
    }: {
        /** The panel's DOM id, which the button's `aria-controls` names. */
        id: string;
        /** What the controls are set to, one or two lines' worth. */
        summary: string;
        /** The muted word before the summary. */
        lead?: string;
        /** Test-id prefix: `{testid}-toggle` on the button, `testid` on the panel. */
        testid: string;
        /** Whether the panel is unfolded below `sm`. */
        open?: boolean;
        class?: ClassValue;
        children: Snippet;
    } = $props();
</script>

<button
    type="button"
    class="flex w-full items-center justify-between gap-2 rounded-box bg-base-200 px-3 py-2 text-left text-sm sm:hidden"
    aria-expanded={open}
    aria-controls={id}
    data-testid="{testid}-toggle"
    onclick={() => (open = !open)}
>
    <!-- Two lines at most: a big interface scale leaves a phone few rem.
         Breaks between words (a long date may break at its hyphen), never
         inside one. A chevron rather than a "Change" label, which took a
         third of the line at 200 %. -->
    <span class="line-clamp-2 min-w-0 [overflow-wrap:break-word]" data-testid="{testid}-summary"
        >{#if lead !== ""}<span class="me-1 text-base-content/60">{lead}</span>{/if}{summary}</span
    >
    <span class="sr-only">{open ? "Done" : "Change"}</span>
    <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 16 16"
        fill="currentColor"
        class={["h-4 w-4 shrink-0 text-base-content/60 transition-transform", open && "rotate-180"]}
        aria-hidden="true"
    >
        <path
            fill-rule="evenodd"
            d="M4.22 6.22a.75.75 0 0 1 1.06 0L8 8.94l2.72-2.72a.75.75 0 1 1 1.06 1.06l-3.25 3.25a.75.75 0 0 1-1.06 0L4.22 7.28a.75.75 0 0 1 0-1.06Z"
            clip-rule="evenodd"
        />
    </svg>
</button>
<div {id} class={[className, "sm:flex", open ? "flex" : "hidden"]} data-testid={testid}>
    {@render children()}
</div>
