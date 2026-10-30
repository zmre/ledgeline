<!-- File text the scenario does not model — `account` and `commodity`
     declarations, transactions, `P`, `include` … (plan 31 C1). Read-only and
     never dragged: the engine writes its own bytes from the file, in its own
     order relative to other file text. Other entries can be moved past it, and
     the list shows exactly where they land. -->
<script lang="ts">
    import type {Entry} from "../../entries";
    import type {FrameProps} from "./frame";

    let {entry, frame, expanded}: {entry: Extract<Entry, {kind: "fixed"}>; frame: FrameProps; expanded: boolean} = $props();

    const lines = $derived(entry.fixed.raw.replace(/\n$/, "").split("\n"));
    /** Long text is clipped to a few lines until opened (Enter, or the button). */
    const CLIP = 3;
    const headId = $props.id();
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<article
    class={["relative scroll-mt-16 rounded-box border border-dashed border-base-content/12 py-1 pr-2 pl-6", frame.cursor && "ring-2 ring-primary/70"]}
    aria-labelledby={headId}
    aria-current={frame.cursor ? "true" : undefined}
    data-testid="projection-entry"
    data-kind="fixed"
    data-key={entry.key}
    data-disabled="false"
    onclick={frame.onActivate}
>
    <div class="flex items-start gap-3">
        <pre class="min-w-0 grow overflow-x-auto font-mono text-xs leading-5 text-base-content/45">{(expanded ? lines : lines.slice(0, CLIP)).join(
                "\n"
            )}{!expanded && lines.length > CLIP ? `\n… ${lines.length - CLIP} more line${lines.length - CLIP === 1 ? "" : "s"}` : ""}</pre>
        <h3
            id={headId}
            class="shrink-0 text-[0.625rem] font-semibold tracking-[0.08em] whitespace-nowrap text-base-content/40 uppercase"
            title="Edit this in the file"
        >
            file text · read-only
        </h3>
    </div>
</article>
