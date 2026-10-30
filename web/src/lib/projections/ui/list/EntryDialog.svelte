<!-- The entry editor: a journal-like modal over one DRAFT (plan 31 C3, C11).

     Built on the journal popup's pattern (`TransactionModal`): a daisyUI
     `modal`, `dismissible` with `trap` (Escape cancels, focus is trapped and
     returns to the Edit button that opened it), Cmd/Ctrl+Enter is Done, and
     the backdrop cancels.

     Every field edits the draft (`entryDraft.svelte.ts`), never the scenario,
     so the readings under the list do not move while the dialog is open.
     Done hands back the scenario with the draft applied, once; Cancel drops it.

     "Writes:" is exact: the piece's own text until the first edit (an
     unchanged piece is written with its own bytes), then the server's render
     of the draft (`POST /api/projections/render`, debounced), which is the
     save's own rendering. A refusal shows the writer's sentence and keeps Done
     shut. A Done pressed while a render is in flight waits for it — the blur
     that commits the last field is what starts that render, so a strictly
     disabled button would swallow the very click that ended the edit. -->
<script lang="ts">
    import {untrack} from "svelte";
    import {LedgelineApi, type RenderProjectionBody} from "#lib/api/native.js";
    import {decodeRenderedEntry} from "#lib/api/nativeDecode.js";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {AmountStyle} from "#lib/domain/types.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {canDisable, entriesOf, plainCopy} from "../../entries";
    import {applyDraft, draftProblems, draftRenderBody, EntryPreview, type Draft} from "../../entryDraft.svelte";
    import type {AssetRow, ForecastDefaults, Scenario} from "../../types";
    import AutoEntryEditor from "./editors/AutoEntryEditor.svelte";
    import CommentEditor from "./editors/CommentEditor.svelte";
    import EventEditor from "./editors/EventEditor.svelte";
    import RuleEditor from "./editors/RuleEditor.svelte";

    let {
        initial,
        scenario,
        declared,
        accountNames,
        commodity,
        styles,
        defaults,
        assetBalances,
        render = renderFromEngine,
        onDone,
        onCancel,
        onJump = null,
    }: {
        /** The draft the dialog opens on. Copied: the dialog owns its own. */
        initial: Draft;
        /** The scenario as committed. Never written here. */
        scenario: Scenario;
        declared: AccountClasses;
        accountNames: string[];
        commodity: string;
        styles: ReadonlyMap<string, AmountStyle>;
        defaults: ForecastDefaults | null;
        assetBalances: ReadonlyMap<string, AssetRow>;
        /** The render call. The engine's, unless a test hands in another. */
        render?: (body: RenderProjectionBody) => Promise<string>;
        /** Done: `next` is the scenario with the draft applied; `key` the entry the cursor should land on. */
        onDone: (next: Scenario, key: string, draft: Draft) => void;
        onCancel: () => void;
        /** Leave the dialog for another step segment (its draft discarded). */
        onJump?: ((group: string) => void) | null;
    } = $props();

    async function renderFromEngine(body: RenderProjectionBody): Promise<string> {
        const url = settings.serverUrl;
        if (url === null) throw new Error("Not connected to the Ledgeline engine.");
        return decodeRenderedEntry(await new LedgelineApi(url).renderProjectionEntry(body)).text;
    }

    // Seeded ONCE from `initial`; the dialog owns its draft after that.
    let draft = $state<Draft>(plainCopy(untrack(() => initial)));
    // One preview per dialog.
    const preview = new EntryPreview((body) => render(body));
    let doneQueued = $state(false);

    const problems = $derived(draftProblems(draft));
    const blocking = $derived(problems.length > 0 || preview.refusal !== null);
    /** The draft's render is on screen, or there is nothing to render (unedited), or the engine can't say (not blocking). */
    const ready = $derived(!draft.edited || preview.rendered || (preview.failure !== null && !preview.pending));

    const kind = $derived(draft.payload.kind);
    const isNew = $derived(draft.origin !== "edit");
    const title = $derived.by(() => {
        const what: Record<typeof kind, string> = {
            rule: draft.payload.kind === "rule" && draft.payload.lines.some((line) => line.role === "asset") ? "asset rule" : "recurring rule",
            event: "one-off",
            auto: "automatic posting",
            comment:
                draft.payload.kind === "comment"
                    ? draft.payload.comment.style === "section"
                        ? "section heading"
                        : draft.payload.comment.style === "block"
                          ? "comment block"
                          : "note"
                    : "",
        };
        switch (draft.origin) {
            case "edit":
                return `Edit ${what[kind]}`;
            case "add":
                return `New ${what[kind]}`;
            case "duplicate":
                return `Copy of a ${what[kind]}`;
            case "alternative":
                return `Alternative to a ${what[kind]}`;
            case "step":
                return "New step in a step change";
        }
    });

    /** The draft's step change, with the other segments' periods, once applied. */
    const step = $derived.by(() => {
        if (draft.payload.kind !== "rule") return null;
        const entry = entriesOf(applyDraft(scenario, draft), declared).find((candidate) => candidate.key === draft.key);
        if (entry?.kind !== "rule" || entry.step === null) return null;
        const others = entry.step.groups
            .filter((group) => group !== draft.key)
            .map((group) => ({
                group,
                period:
                    scenario.lines.find((line) => line.group === group)?.period.raw ??
                    (draft.stepBefore?.kind === "rule" ? (draft.stepBefore.lines[0]?.period.raw ?? "") : ""),
            }));
        return {...entry.step, others};
    });

    // Start the preview: the piece's own text, or (new, or never rendered) the server's render.
    // Read once, at open.
    if (!draft.edited && draft.item.raw !== "") preview.show(draft.item.raw);
    else refresh();

    function refresh(): void {
        if (draftProblems(draft).length > 0) {
            preview.hold();
            return;
        }
        preview.request(draftRenderBody(scenario, draft));
    }

    /** A field was committed. */
    function edited(): void {
        draft.edited = true;
        refresh();
    }

    function done(): void {
        if (blocking) return;
        if (!draft.edited) {
            // Nothing changed: nothing to apply, no run to start.
            onCancel();
            return;
        }
        if (!ready) {
            doneQueued = true;
            return;
        }
        onDone(applyDraft(scenario, draft, preview.rendered ? preview.text : null), draft.key, draft);
    }

    // A Done pressed while the render was in flight lands once it answers —
    // or stands down if the answer is a refusal.
    $effect(() => {
        if (!doneQueued || preview.pending) return;
        doneQueued = false;
        if (!blocking && ready) done();
    });

    function cancel(): void {
        preview.stop();
        onCancel();
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== "Enter" || !(event.metaKey || event.ctrlKey)) return;
        // A completion popup takes precedence: it has already prevented this.
        if (event.defaultPrevented) return;
        event.preventDefault();
        // Commit the focused field first: its change event is what records it.
        (document.activeElement as HTMLElement | null)?.blur();
        done();
    }

    $effect(() => () => preview.stop());

    const headingId = $props.id();
</script>

<div
    class="modal modal-open modal-bottom bg-black/55 backdrop-blur-[1.5px] sm:modal-middle"
    role="dialog"
    aria-modal="true"
    aria-labelledby={headingId}
    tabindex="-1"
    data-testid="entry-dialog"
    onkeydown={onKeydown}
    use:dismissible={{trap: true, onDismiss: cancel}}
>
    <div class="modal-box flex max-h-[92vh] max-w-3xl flex-col gap-0 overflow-hidden border border-base-content/10 bg-base-200 p-0 shadow-2xl">
        <header class="flex items-baseline gap-3 border-b border-base-content/8 px-5 pt-4 pb-3">
            <h2 id={headingId} class="text-base font-semibold tracking-tight">{title}</h2>
            {#if canDisable(draft.item.kind)}
                <label class="ml-auto flex cursor-pointer items-center gap-2 text-sm">
                    <span class={draft.item.disabled ? "text-base-content/50" : "text-base-content/80"}>{draft.item.disabled ? "Turned off" : "Enabled"}</span>
                    <input
                        type="checkbox"
                        class="toggle toggle-primary toggle-sm"
                        checked={!draft.item.disabled}
                        aria-label="Enabled"
                        onchange={(e) => {
                            draft.item.disabled = !e.currentTarget.checked;
                            edited();
                        }}
                    />
                </label>
            {/if}
        </header>

        <div class="flex flex-col gap-4 overflow-y-auto px-5 py-4">
            {#if draft.disableOriginal !== null}
                <p class="rounded-box bg-warning/8 px-3 py-2 text-xs text-warning/90">Done adds this copy and turns the original off, in one change.</p>
            {/if}
            {#if canDisable(draft.item.kind)}
                <label class="flex flex-col gap-1">
                    <span class="text-xs text-base-content/60">Comment above this entry</span>
                    <textarea
                        class="textarea min-h-0 w-full font-mono text-sm placeholder:font-sans"
                        rows={Math.max(1, draft.attached.split("\n").length)}
                        value={draft.attached}
                        placeholder="Optional. Written as ; lines above the entry, and moves with it."
                        aria-label="Comment above this entry"
                        onchange={(e) => {
                            draft.attached = e.currentTarget.value;
                            edited();
                        }}></textarea>
                </label>
            {/if}

            {#if draft.payload.kind === "rule"}
                <RuleEditor
                    lines={draft.payload.lines}
                    group={draft.key}
                    {accountNames}
                    {declared}
                    {styles}
                    {defaults}
                    {assetBalances}
                    cash={scenario.cash}
                    {step}
                    onEdit={edited}
                    onJump={isNew || onJump === null ? null : onJump}
                />
            {:else if draft.payload.kind === "event"}
                <EventEditor event={draft.payload.event} {accountNames} {commodity} {defaults} cash={scenario.cash} onEdit={edited} />
            {:else if draft.payload.kind === "auto"}
                <AutoEntryEditor rule={draft.payload.rule} {scenario} {accountNames} {commodity} {styles} onEdit={edited} />
            {:else}
                <CommentEditor comment={draft.payload.comment} onEdit={edited} />
            {/if}

            <section class="flex flex-col gap-1" aria-label="What a save writes">
                <div class="flex items-baseline justify-between gap-2">
                    <span class="text-xs font-medium text-base-content/70"
                        >Writes <span class="font-normal text-base-content/45">— exact, from the server</span></span
                    >
                    {#if preview.pending}
                        <span class="flex items-center gap-1 text-xs text-base-content/45"
                            ><span class="loading loading-xs loading-spinner"></span>rendering</span
                        >
                    {/if}
                </div>
                <pre
                    class={[
                        "max-h-48 min-h-12 overflow-auto rounded-box border border-base-content/8 bg-base-100 px-3 py-2 font-mono text-xs leading-5 whitespace-pre text-base-content/85 transition-opacity",
                        preview.pending && "opacity-50",
                    ]}
                    data-testid="entry-dialog-writes">{preview.text === "" ? "" : preview.text.replace(/\n$/, "")}</pre>
                {#if preview.text === "" && !preview.pending && preview.refusal === null}
                    <p class="text-xs text-base-content/45">Fill in the fields to see the text this writes.</p>
                {/if}
                <div aria-live="polite">
                    {#if preview.refusal !== null}
                        <p class="text-xs text-error" role="alert" data-testid="entry-dialog-refusal">{preview.refusal}</p>
                    {:else if preview.failure !== null}
                        <p class="text-xs text-warning" data-testid="entry-dialog-failure">Couldn't preview the text: {preview.failure}</p>
                    {/if}
                </div>
            </section>

            {#if problems.length > 0 && draft.edited}
                <ul class="list-inside list-disc text-xs text-warning" data-testid="entry-dialog-problems">
                    {#each problems as problem (problem)}
                        <li>{problem}</li>
                    {/each}
                </ul>
            {/if}
        </div>

        <footer class="flex flex-wrap items-center gap-2 border-t border-base-content/8 px-5 py-3">
            <span class="text-xs text-base-content/45">{isNew ? "Nothing is added until you press Done." : "Changes apply when you press Done."}</span>
            <span class="grow"></span>
            <button type="button" class="btn btn-ghost btn-sm" onclick={cancel} data-testid="entry-dialog-cancel">Cancel</button>
            <button
                type="button"
                class="btn btn-primary btn-sm"
                disabled={blocking}
                aria-busy={doneQueued}
                onclick={done}
                data-testid="entry-dialog-done"
                title="Done (⌘↵)"
            >
                {#if doneQueued}<span class="loading loading-xs loading-spinner"></span>{/if}
                Done <span class="hidden font-normal opacity-70 sm:inline" aria-hidden="true">⌘↵</span>
            </button>
        </footer>
    </div>
    <button type="button" class="modal-backdrop" aria-label="Cancel" onclick={cancel}>close</button>
</div>
