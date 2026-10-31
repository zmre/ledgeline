<script lang="ts">
    // The review overlay (plan 32 Phase 3): one transaction at a time, who and
    // how much front and centre, keyboard first.
    //
    // # Two modes (D16)
    //
    // CARD mode is where every card lands — focus on the card itself, not a
    // field — and single keys act (`y`, `→`, `d`, `1`…). FIELD mode is typing:
    // the keymap ignores keys in fields by rule, so single keys type. Escape
    // walks back out: field → card → close (asking first when the card has
    // unsaved edits). ⌘↵ and ⌘⇧↵ work from both, because this element handles
    // them itself (the TransactionModal rule) before the keymap sees them.
    //
    // # Keys
    //
    // The overlay is the app's first `PRIORITY.overlay`, `modal: true` layer:
    // nothing below it — the journal's j/k/x — can fire while it is up. Its
    // bindings are the "Review" group of the `?` sheet; `?` itself is bound here
    // because the modal layer blinds the global one.
    //
    // # The deck (D29)
    //
    // A full-viewport layer over a backdrop: a slim top bar, and under it a
    // deck of cards stacked in one grid cell. The top card is self-contained
    // (hero, fields, Similar and its own footer) and scrolls inside itself, so
    // nothing between it and the viewport clips it. Only the layer is
    // `overflow: hidden`. The next two queue items wait beneath it as peeks
    // (hero only, `inert`, `aria-hidden`). At the end of the queue the run's
    // summary is what lies under the last card.
    //
    // `{#key}` on the top card: the next card mounts the moment the store
    // advances, rising out of the peek slot while the old one flies off the
    // viewport ABOVE it (or, on ←, settles back into the peek slot BELOW the
    // card coming back). Svelte marks the outgoing card `inert`, and the styles
    // below take it out of the grid's flow. Keys act on the new card at once,
    // and any keypress first finishes whatever is still moving (`finishMotion`).
    // Two cards plus the peeks can be in the DOM together, so every lookup goes
    // through `currentCard()`. Reduced motion is a short cross-fade with no
    // travel, and the peeks still show.
    //
    // # Saves (D14)
    //
    // Optimistic: `review.save` advances at once and writes behind; the header's
    // dot says "saving…". A failure rewinds to that card with its edits and the
    // error.
    import {flushSync, tick, untrack} from "svelte";
    import {cardChange, cardOps, isSplit, reviewFormOf} from "#lib/api/editMapping.js";
    import {openAutoRule} from "#lib/autoRules/openRule.js";
    import {txnModal} from "#lib/journal/edit/modalState.svelte.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {keymap, registerKeys} from "#lib/keys/keymap.svelte.js";
    import {isTypingTarget, type TargetLike} from "#lib/keys/target.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {review, type ReviewCard} from "#lib/stores/review.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {acceptChip, changeSummary, chipsFor, cycleChip, footerState, hasUnknown, isDirty, similarRows, type Chips} from "../card";
    import type {Transaction} from "#lib/domain/types.js";
    import type {QueueKind, ReviewForm} from "../types";
    import {reviewLaunch} from "./launch.svelte";
    import {
        backdrop,
        barIn,
        cardIn,
        cardOut,
        finishMotion,
        layerOut,
        PEEK_DEPTH,
        peekIn,
        peekOut,
        peekStyle,
        prefersReducedMotion,
        stampPop,
        type CardExit,
        type MotionParams,
    } from "./motion";
    import ReviewFields from "./ReviewFields.svelte";
    import ReviewFooter from "./ReviewFooter.svelte";
    import ReviewHero from "./ReviewHero.svelte";
    import ReviewIcon from "./ReviewIcon.svelte";
    import ReviewSettings from "./ReviewSettings.svelte";
    import SimilarSection from "./SimilarSection.svelte";

    let {kind}: {kind: QueueKind} = $props();

    const TITLES: Record<QueueKind, string> = {unknowns: "Review Unknowns", recents: "Review Recents"};
    const NO_CHIPS: Chips = {chips: [], target: null, initial: null, mode: "fix"};
    /**
     * A card's surface in the deck. Every card sits in the one grid cell, pinned
     * at its bottom edge, so the peeks' shrink-and-drop (motion.ts `PEEKS`)
     * leaves a strip of each showing below the card above it.
     */
    const SURFACE = "relative min-w-0 origin-bottom overflow-hidden rounded-box border bg-base-100 [grid-area:1/1]";
    /** The top card. Its body scrolls inside it, under a max height that leaves room for the bar and the peeks' strips. */
    const CARD = `deck-card ${SURFACE} pointer-events-auto z-[5] flex max-h-[calc(100dvh-7.5rem)] flex-col self-start shadow-2xl outline-none`;
    /** Loading, the error, nothing to review, and the run's summary under the last card. */
    const PANEL = `${SURFACE} pointer-events-auto z-0 self-start border-base-content/10 shadow-2xl`;
    /** A card waiting beneath: as tall as the top card (stretched to the cell), hero only. */
    const PEEK = `${SURFACE} pointer-events-none self-stretch border-base-content/10 p-5 shadow-xl sm:p-8`;
    const today = localToday();
    const reduced = prefersReducedMotion();
    /**
     * The moment the NEXT card transition belongs to, read by `cardOut`/`cardIn`
     * when they start. "open" until the first action, "close" once closing. A
     * plain object on purpose: it is set right before the store advances, and
     * nothing should re-render because of it.
     */
    const motion: MotionParams = {exit: "open", reduced};

    let root = $state<HTMLDivElement | null>(null);
    let stack = $state<HTMLDivElement | null>(null);
    let similarSection = $state<{toggleAll: () => void} | null>(null);
    let form = $state<ReviewForm>({date: today, description: "", comment: "", tags: [], postings: []});
    let selected = $state<number | null>(null);
    let similarOpen = $state(false);
    let checked = $state<number[]>([]);
    let showDate = $state(false);
    let confirmClose = $state(false);
    let menuOpen = $state(false);
    let settingsOpen = $state(false);
    /** The card showing the ✓ stamp as it leaves. */
    let stamped = $state<number | null>(null);
    /** The last moment that animated — exposed as `data-exit` so a test can see it. */
    let lastExit = $state<CardExit | null>(null);
    /** The last card saved: where the journal's cursor lands on close. */
    let lastSaved: number | null = null;
    /** `${index}:${fingerprint}` the form was seeded from, and that transaction. */
    let seededFrom: string | null = null;
    let seededTxn: Transaction | null = null;

    const classes = $derived(journal.accountClasses);
    const current = $derived(review.current);
    const txn = $derived(current?.item.txn ?? null);
    const original = $derived(txn === null ? null : reviewFormOf(txn));
    const chips = $derived(current !== null && original !== null ? chipsFor(current.item, original, classes) : NO_CHIPS);
    const dirty = $derived(txn !== null && isDirty(txn, form));
    const unknown = $derived(txn !== null && hasUnknown(txn, form, classes));
    const phase = $derived(footerState(dirty, unknown));
    const split = $derived(txn !== null && isSplit(txn, form));
    const change = $derived(txn === null ? null : cardChange(txn, form));
    const similarSet = $derived(txn === null ? null : review.similar(txn.index));
    const rows = $derived(similarSet === null || split ? [] : similarRows(similarSet.candidates, similarSet.inQueue, change));
    const updatedFrom = $derived.by(() => {
        const from = current?.updatedFrom;
        if (from === null || from === undefined) return null;
        const at = review.cards.findIndex((card) => card.item.txn.index === from);
        return at < 0 ? null : at + 1;
    });
    const finished = $derived(!review.loading && review.error === null && review.count > 0 && current === null);
    const empty = $derived(!review.loading && review.error === null && review.count === 0);
    const failure = $derived(review.failure !== null && txn !== null && review.failure.index === txn.index ? review.failure.failure.message : null);
    const progress = $derived(review.count === 0 ? 0 : Math.min(1, review.position / review.count));
    /** The cards waiting beneath the top one, nearest first. None past the last card: the summary lies there. */
    const peeks = $derived(current === null ? [] : review.cards.slice(review.cursor + 1, review.cursor + 1 + PEEK_DEPTH));

    // One run per mount: the page mounts this when a run opens and unmounts it when it closes.
    untrack(() => {
        void review.open(kind, kind === "recents" ? {limit: settings.reviewRecentLimit, maxDays: settings.reviewMaxDays} : {maxDays: settings.reviewMaxDays});
    });

    // Each new card: seed the form from its draft (or its transaction), reset
    // what is per-card, and put focus on the CARD. Keyed on index AND
    // fingerprint, so a card a similar-apply or a revalidation refreshed
    // underneath re-seeds — unless the user is mid-edit on it.
    $effect(() => {
        const card = current;
        const key = card === null ? null : `${card.item.txn.index}:${card.item.fingerprint ?? ""}`;
        untrack(() => {
            if (key === seededFrom) return;
            const sameCard = card !== null && seededFrom !== null && seededFrom.startsWith(`${card.item.txn.index}:`);
            seededFrom = key;
            if (card === null) {
                void focusCard();
                return;
            }
            // The user's edits are judged against the text they were made on,
            // not the refreshed one (which differs from the form by definition).
            if (sameCard && seededTxn !== null && isDirty(seededTxn, form)) return;
            seed(card);
            if (!sameCard) void focusCard();
        });
    });

    // The run follows the journal: a change made elsewhere re-validates the
    // queue (D5a), and the completion lists follow the generation (D15).
    $effect(() => {
        const generation = journal.generation;
        untrack(() => {
            review.syncGeneration(generation);
            void reviewLaunch.loadVocabulary(generation);
        });
    });

    function seed(card: ReviewCard): void {
        seededTxn = card.item.txn;
        form = card.draft === null ? reviewFormOf(card.item.txn) : structuredClone($state.snapshot(card.draft) as ReviewForm);
        selected = chipsFor(card.item, reviewFormOf(card.item.txn), classes).initial;
        similarOpen = false;
        checked = [];
        showDate = false;
        confirmClose = false;
        stamped = null;
    }

    /**
     * The card on screen. While one exits, two cards are in the DOM, so every
     * lookup of a field goes through this, never the whole overlay: a bare
     * `querySelector` finds the outgoing card's field first.
     */
    function currentCard(): HTMLElement | null {
        const index = review.current?.item.txn.index;
        return index === undefined ? null : (stack?.querySelector<HTMLElement>(`[data-card-index="${index}"]`) ?? null);
    }

    async function focusCard(): Promise<void> {
        await tick();
        const target = review.current === null ? stack?.querySelector<HTMLElement>("[data-review-summary]") : currentCard();
        (target ?? root)?.focus({preventScroll: true});
    }

    function focusField(name: string): void {
        (currentCard() ?? root)?.querySelector<HTMLInputElement>(`[data-review-field="${name}"] input`)?.focus();
    }

    /** Remember the card's edits as it is left (D5: they survive ← and →). */
    function stash(): void {
        if (current !== null) review.setDraft(dirty ? ($state.snapshot(form) as ReviewForm) : null);
    }

    function exitAs(exit: CardExit): void {
        finishMotion(stack);
        motion.exit = exit;
        lastExit = exit;
    }

    // -- actions -----------------------------------------------------------

    /** The left footer button, ⌘↵, `y`, ↵: Reviewed, or the dirty state's save. On the summary, close. */
    function primary(): void {
        if (current === null || txn === null) {
            if (finished || empty) close();
            return;
        }
        if (phase === "clean-unknown" || current.status === "saving") return;
        const reviewedOn = phase === "dirty-unknown" ? null : today;
        const ops = cardOps(txn, form, {key: current.item.key, reviewedOn, defaultCommodity: journal.dominantCommodity});
        if (ops.length === 0) return;
        const picked = new Set(checked);
        const similar = change === null ? [] : rows.filter((row) => picked.has(row.txn.txn.index)).map((row) => ({txn: row.txn, ops: row.ops}));
        const index = txn.index;
        // The stamp must be ON the card before it starts to leave: once its
        // block is outroing, it no longer updates.
        if (reviewedOn !== null) flushSync(() => (stamped = index));
        exitAs(reviewedOn !== null ? "reviewed" : "saved");
        lastSaved = index;
        // Kept as the card's draft until the write lands: a failure rewinds
        // to this card with exactly these edits (D14).
        review.setDraft($state.snapshot(form) as ReviewForm);
        void review.save(ops, similar);
    }

    /** ⌘⇧↵ / `Y`: take the selected suggestion, then do what ⌘↵ does. With no chip selected it IS ⌘↵. */
    function accept(): void {
        if (selected !== null) pick(selected);
        primary();
    }

    /** Fill chip `at` into its row (and its rename, D13a), and select it. In place, so a focused field sees it at once. */
    function pick(at: number): void {
        const chip = chips.chips[at];
        if (txn === null || chips.target === null || chip === undefined) return;
        const next = acceptChip(txn, form, chips.target, chip);
        const row = form.postings[chips.target];
        if (row !== undefined) row.account = next.postings[chips.target]?.account ?? row.account;
        form.description = next.description;
        selected = at;
    }

    /** Tab on the chip row's account field. False lets the Tab through (the user typed their own account). */
    function tabChip(fieldValue: string): boolean {
        const value = fieldValue.trim();
        const untouched = value === "" || value === original?.postings[chips.target ?? -1]?.account || chips.chips.some((chip) => chip.account === value);
        if (!untouched) return false;
        const next = cycleChip(chips.chips, selected, value);
        if (next === null) return false;
        pick(next);
        return true;
    }

    function skip(): void {
        if (current === null) return;
        stash();
        exitAs("skip");
        review.skip();
    }

    function back(): void {
        if (review.cursor === 0) return;
        stash();
        exitAs("back");
        review.back();
    }

    function revert(): void {
        if (current === null) return;
        review.setDraft(null);
        seed({...current, draft: null});
        void focusCard();
    }

    function ignore(): void {
        if (current === null || current.status === "saving") return;
        menuOpen = false;
        exitAs("ignore");
        lastSaved = current.item.txn.index;
        void review.ignore();
    }

    function openEditor(): void {
        if (txn === null) return;
        const index = txn.index;
        close();
        txnModal.openEdit(index);
    }

    function openRule(rule: number): void {
        close();
        void openAutoRule(rule);
    }

    async function toggleSimilar(): Promise<void> {
        if (rows.length === 0) return;
        similarOpen = !similarOpen;
        if (!similarOpen) return void focusCard();
        await tick();
        (currentCard() ?? root)?.querySelector<HTMLInputElement>("[data-testid='review-similar-rows'] input:not([disabled])")?.focus();
    }

    function editDate(): void {
        menuOpen = false;
        showDate = true;
        void tick().then(() => focusField("date"));
    }

    function editAccount(at: number | null): void {
        const row = at ?? Math.max(0, chips.mode === "fix" && chips.target !== null ? chips.target : 0);
        if (row < form.postings.length) focusField(`account-${row}`);
    }

    function close(): void {
        confirmClose = false;
        // The layer fades and drops as one; the cards do nothing of their own,
        // and the outgoing one stays in the grid's flow (see the styles).
        finishMotion(stack);
        motion.exit = "close";
        root?.setAttribute("data-closing", "");
        review.close();
        reviewLaunch.close(lastSaved);
    }

    /** Escape in card mode, ✕, the backdrop: close — but ask first if the card has unsaved edits. */
    function requestClose(): void {
        if (confirmClose) {
            confirmClose = false;
            return void focusCard();
        }
        if (dirty) {
            confirmClose = true;
            return;
        }
        close();
    }

    /** The click a focused control would have had (the keymap prevents the native one for ↵ and Space). */
    function activateFocused(): boolean {
        const focused = document.activeElement;
        if (focused instanceof HTMLButtonElement || (focused instanceof HTMLInputElement && focused.type === "checkbox")) {
            if (root?.contains(focused) === true && !focused.hasAttribute("data-review-card")) {
                focused.click();
                return true;
            }
        }
        return false;
    }

    // Element-level, so it runs BEFORE the window keymap and before
    // `dismissible`'s document listener — and after any field's own handler,
    // which claims its keys with `preventDefault` (a combobox's Enter/Escape).
    function onKeydown(event: KeyboardEvent): void {
        // D29: a keystroke mid-animation finishes it, then acts on the new card.
        finishMotion(stack);
        if (event.defaultPrevented || event.isComposing) return;
        if (event.key === "Escape" && isTypingTarget(event.target as TargetLike | null)) {
            event.preventDefault();
            void focusCard();
            return;
        }
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.altKey) {
            event.preventDefault();
            if (event.shiftKey) accept();
            else primary();
        }
    }

    // -- the key layer -----------------------------------------------------

    const live = (): boolean => !keymap.helpOpen && !confirmClose;
    const onCard = (): boolean => live() && review.current !== null;

    registerKeys({
        id: "review-overlay",
        priority: PRIORITY.overlay,
        modal: true,
        bindings: [
            {keys: "mod+Enter", label: "Reviewed (or save the changes)", group: "Review", enabled: live, run: primary},
            {keys: "mod+shift+Enter", label: "Accept the suggestion, mark reviewed, next", group: "Review", enabled: onCard, run: accept},
            {keys: "y", label: "Reviewed (or save the changes)", group: "Review", enabled: live, run: primary},
            {keys: "Enter", label: "Reviewed (or save the changes)", group: "Review", enabled: live, run: () => activateFocused() || primary()},
            {keys: "Y", label: "Accept the suggestion, mark reviewed, next", group: "Review", enabled: onCard, run: accept},
            {keys: "ArrowRight", label: "Skip", group: "Review", enabled: onCard, run: skip},
            {keys: "n", label: "Skip", group: "Review", enabled: onCard, run: skip},
            {keys: "l", label: "Skip", group: "Review", enabled: onCard, run: skip},
            {keys: "ArrowLeft", label: "Previous card", group: "Review", enabled: live, run: back},
            {keys: "p", label: "Previous card", group: "Review", enabled: live, run: back},
            {keys: "h", label: "Previous card", group: "Review", enabled: live, run: back},
            {keys: "z", label: "Revert changes", group: "Review", enabled: onCard, run: revert},
            {keys: "d", label: "Edit description", group: "Review", enabled: onCard, run: () => focusField("description")},
            {keys: "a", label: "Edit the unknown account", group: "Review", enabled: onCard, run: () => editAccount(null)},
            ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => ({
                keys: String(n),
                label: `Edit posting ${n}'s account`,
                group: "Review" as const,
                enabled: () => onCard() && n <= form.postings.length,
                run: () => editAccount(n - 1),
            })),
            {keys: "$", label: "Edit the first amount", group: "Review", enabled: onCard, run: () => focusField("amount-0")},
            {keys: "t", label: "Add a tag", group: "Review", enabled: onCard, run: () => focusField("tags")},
            {keys: "m", label: "Edit comment", group: "Review", enabled: onCard, run: () => focusField("comment")},
            {keys: "s", label: "Open or close Similar", group: "Review", enabled: onCard, run: () => void toggleSimilar()},
            {keys: " ", label: "Toggle the focused similar row", group: "Review", enabled: () => onCard() && similarOpen, run: () => void activateFocused()},
            {keys: "A", label: "Check or uncheck all similar", group: "Review", enabled: onCard, run: () => similarSection?.toggleAll()},
            {keys: "i", label: "Ignore (never ask again)", group: "Review", enabled: onCard, run: ignore},
            {keys: "o", label: "Open in the full editor", group: "Review", enabled: onCard, run: openEditor},
            {keys: "Escape", label: "Leave the field, or close", group: "Review", run: requestClose},
            {keys: "?", label: "Show keyboard shortcuts", group: "Review", run: () => keymap.toggleHelp()},
        ],
    });
</script>

<div
    bind:this={root}
    class="review-layer fixed inset-0 z-[999] overflow-hidden"
    role="dialog"
    aria-modal="true"
    aria-label={TITLES[kind]}
    data-testid="review-overlay"
    tabindex="-1"
    onkeydown={onKeydown}
    use:dismissible={{trap: true, onDismiss: requestClose}}
>
    <!-- Bar and deck float on the backdrop; empty space passes clicks through to it. -->
    <div class="pointer-events-none relative z-10 flex h-full flex-col items-center gap-3 px-3 pt-3 sm:gap-4 sm:px-6 sm:pt-5" out:layerOut|global={{reduced}}>
        <!-- Top bar: queue, position, progress, saving, ⋯, ✕ -->
        <div
            class="pointer-events-auto flex w-full max-w-4xl shrink-0 items-center gap-3 rounded-box border border-base-content/10 bg-base-300/95 py-1.5 pr-1.5 pl-4 text-base-content shadow-lg backdrop-blur"
            in:barIn|global={{reduced}}
        >
            <h2 class="text-sm font-semibold tracking-wide">{TITLES[kind]}</h2>
            {#if review.count > 0}
                <span class="text-sm text-base-content/65 tabular-nums" data-testid="review-position" aria-live="polite">
                    {review.position} of {review.count}
                </span>
                <div
                    class="h-1 w-28 overflow-hidden rounded-full bg-base-content/15"
                    role="progressbar"
                    aria-valuemin={0}
                    aria-valuemax={review.count}
                    aria-valuenow={review.position}
                    aria-label="Progress"
                >
                    <div
                        class="h-full origin-left rounded-full bg-success transition-transform duration-300 ease-out motion-reduce:transition-none"
                        style="transform: scaleX({progress})"
                    ></div>
                </div>
            {/if}
            {#if review.saving > 0}
                <span class="flex items-center gap-1.5 text-xs text-base-content/60" data-testid="review-saving" role="status">
                    <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-success"></span> saving…
                </span>
            {/if}
            <span class="grow"></span>
            <details
                class="dropdown dropdown-end"
                bind:open={menuOpen}
                use:dismissible={{active: menuOpen, outside: true, onDismiss: () => (menuOpen = false)}}
            >
                <summary class="btn btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10" aria-label="More actions" title="More actions">
                    <ReviewIcon name="ellipsis" />
                </summary>
                <div class="dropdown-content z-30 mt-1 rounded-box border border-base-300 bg-base-200 p-2 text-base-content shadow-xl">
                    {#if settingsOpen}
                        <ReviewSettings />
                        <button type="button" class="btn mt-2 w-full btn-ghost btn-xs" onclick={() => (settingsOpen = false)}>Done</button>
                    {:else}
                        <ul class="menu w-60 p-0 text-sm">
                            <li>
                                <button type="button" onclick={ignore} disabled={current === null}>
                                    <ReviewIcon name="ban" /> Ignore — never ask again <kbd class="ml-auto kbd kbd-xs">i</kbd>
                                </button>
                            </li>
                            <li>
                                <button type="button" onclick={openEditor} disabled={current === null}>
                                    <ReviewIcon name="pencil" /> Open in full editor <kbd class="ml-auto kbd kbd-xs">o</kbd>
                                </button>
                            </li>
                            <li>
                                <button type="button" onclick={editDate} disabled={current === null}>Edit date</button>
                            </li>
                            <li>
                                <button type="button" onclick={() => (settingsOpen = true)}><ReviewIcon name="sliders" /> Settings</button>
                            </li>
                        </ul>
                    {/if}
                </div>
            </details>
            <button
                type="button"
                class="btn btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10"
                aria-label="Close review"
                title="Close (Esc)"
                onclick={requestClose}
            >
                <ReviewIcon name="x" />
            </button>
        </div>

        <!-- The deck: every card in one grid cell. Summary at the bottom, then the peeks, then the top card. -->
        <div
            bind:this={stack}
            class="deck relative grid w-full max-w-4xl"
            data-testid="review-stack"
            data-exit={lastExit}
            data-motion={reduced ? "reduced" : "full"}
        >
            {#if review.loading}
                <div class="{PANEL} flex items-center justify-center gap-2 py-16 text-sm text-base-content/60" role="status">
                    <span class="loading loading-sm loading-spinner"></span> Loading the queue…
                </div>
            {:else if review.error !== null}
                <div class="{PANEL} p-5 sm:p-8" role="alert">
                    <div class="alert alert-error"><span>Couldn't load the queue: {review.error}</span></div>
                </div>
            {:else if empty}
                <div
                    class="{PANEL} flex flex-col items-center gap-3 px-5 py-14 text-center outline-none"
                    data-testid="review-empty"
                    data-review-summary
                    tabindex="-1"
                    in:cardIn|global={motion}
                >
                    <span class="grid h-12 w-12 place-items-center rounded-full bg-success/15 text-success"><ReviewIcon name="check" class="h-6 w-6" /></span>
                    <p class="text-lg font-medium">Nothing to review</p>
                    <p class="text-sm text-base-content/60">
                        {kind === "unknowns" ? "Every transaction in the window has a category." : "Everything in the window is reviewed."}
                        {#if review.notEditable > 0}({review.notEditable} not editable){/if}
                    </p>
                    <button type="button" class="btn btn-sm" onclick={close}>Close</button>
                </div>
            {:else}
                {#if finished}
                    <!-- What lies under the last card: it rises out of the peek slot as that card leaves. -->
                    <div
                        class="{PANEL} flex flex-col items-center gap-3 px-5 py-12 text-center outline-none"
                        data-testid="review-summary"
                        data-review-summary
                        tabindex="-1"
                        in:cardIn={motion}
                        out:peekOut={{reduced}}
                    >
                        <span class={["grid h-14 w-14 place-items-center rounded-full bg-success/15 text-success", !reduced && "review-pulse"]}>
                            <ReviewIcon name="check" class="h-7 w-7" />
                        </span>
                        <p class="text-lg font-medium">Run finished</p>
                        <p class="text-sm text-base-content/70 tabular-nums" data-testid="review-tally">
                            {review.tally.saved} reviewed · {review.tally.fixed} fixed via similar · {review.tally.skipped} skipped
                        </p>
                        <button type="button" class="btn btn-sm btn-success" onclick={close}>Done <kbd class="kbd kbd-xs">↵</kbd></button>
                    </div>
                {/if}

                {#each peeks as waiting, at (waiting.item.txn.index)}
                    <div
                        class={[PEEK, at === 0 ? "z-[2]" : "z-[1]"]}
                        style={peekStyle(at + 1, reduced)}
                        inert
                        aria-hidden="true"
                        data-testid="review-peek"
                        data-peek-index={waiting.item.txn.index}
                        in:peekIn|global={{reduced, opening: motion.exit === "open"}}
                        out:peekOut={{reduced}}
                    >
                        <ReviewHero txn={waiting.item.txn} {classes} peek />
                    </div>
                {/each}

                {#key txn?.index}
                    {#if txn !== null && current !== null}
                        <article
                            class={[CARD, reduced && stamped === txn.index ? "border-success" : "border-base-content/10"]}
                            data-review-card
                            data-card-index={txn.index}
                            data-testid="review-card"
                            tabindex="-1"
                            aria-label="{txn.date} {txn.description}"
                            in:cardIn|global={motion}
                            out:cardOut|global={motion}
                        >
                            <!-- The card's body scrolls inside the card, so the deck never clips it. -->
                            <div class="flex min-h-0 flex-col gap-6 overflow-y-auto overscroll-contain p-5 sm:p-8">
                                <ReviewHero {txn} {classes} {updatedFrom} />
                                <div class="border-t border-dashed border-base-content/15"></div>
                                {#if failure !== null}
                                    <div class="alert py-2 text-sm alert-error" role="alert" data-testid="review-failure">
                                        <span class="grow break-words">Not saved: {failure}</span>
                                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => review.clearFailure()}>Dismiss</button>
                                    </div>
                                {/if}
                                <ReviewFields
                                    bind:form
                                    {txn}
                                    {classes}
                                    accountNames={journal.accountNames}
                                    payees={reviewLaunch.payees}
                                    tags={reviewLaunch.tags}
                                    {today}
                                    {chips}
                                    {selected}
                                    {showDate}
                                    defaultCommodity={journal.dominantCommodity}
                                    disabled={current.status === "saving"}
                                    onTabChip={tabChip}
                                    onPickChip={pick}
                                    onRule={openRule}
                                />
                                {#if split}
                                    <p class="text-xs text-base-content/50" data-testid="review-split-note">Splits aren't applied to similar transactions.</p>
                                {:else if rows.length > 0}
                                    <SimilarSection bind:this={similarSection} {rows} bind:checked bind:open={similarOpen} applying={changeSummary(change)} />
                                {/if}
                            </div>

                            {#if confirmClose}
                                <div
                                    class="flex min-h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-warning/30 bg-warning/10 px-5 py-3 text-sm sm:px-8"
                                    role="alert"
                                    data-testid="review-confirm-close"
                                >
                                    <span>This card has unsaved changes.</span>
                                    <span class="flex gap-2">
                                        <button type="button" class="btn btn-ghost btn-sm" onclick={() => ((confirmClose = false), void focusCard())}
                                            >Keep editing</button
                                        >
                                        <button type="button" class="btn btn-sm btn-warning" onclick={close}>Discard and close</button>
                                    </span>
                                </div>
                            {:else}
                                <ReviewFooter {phase} {reduced} busy={current.status === "saving"} onPrimary={primary} onSkip={skip} onRevert={revert} />
                            {/if}

                            {#if stamped === txn.index}
                                <div
                                    class="pointer-events-none absolute top-6 right-8 grid h-24 w-24 -rotate-12 place-items-center rounded-full border-4 border-double border-success/80 text-success"
                                    data-testid="review-stamp"
                                    aria-hidden="true"
                                    in:stampPop={{reduced}}
                                >
                                    <span class="flex flex-col items-center text-[0.65rem] leading-none font-bold tracking-[0.2em]">
                                        <ReviewIcon name="check" class="mb-1 h-7 w-7" />REVIEWED
                                    </span>
                                </div>
                            {/if}
                        </article>
                    {/if}
                {/key}
            {/if}
        </div>
    </div>
    <button
        type="button"
        class="absolute inset-0 z-0 h-full w-full cursor-default bg-black/70 backdrop-blur-sm"
        aria-label="Close review"
        onclick={requestClose}
        in:backdrop|global={{reduced}}
        out:backdrop|global={{reduced, closing: true}}
    ></button>
</div>

<style>
    /* The outgoing top card (Svelte marks an outroing element inert). It leaves
       the grid's flow, so the cell, and the peeks stretched to it, take the
       incoming card's height at once. It sits above the incoming card, which it
       uncovers as it flies. Not while the layer closes: then nothing moves, and
       the card stays put. */
    .review-layer:not([data-closing]) :global(.deck-card[inert]) {
        position: absolute;
        inset-inline: 0;
        top: 0;
        z-index: 6;
    }
    /* ...except on ←, where the card coming back lands on top of it, and it
       settles into the peek slot beneath. */
    .review-layer:not([data-closing]) .deck[data-exit="back"] :global(.deck-card[inert]) {
        z-index: 3;
    }

    /* The summary's ✓: one brief pulse (D29). */
    .review-pulse {
        animation: review-pulse 420ms ease-out 1;
    }
    @keyframes review-pulse {
        0% {
            transform: scale(0.8);
        }
        55% {
            transform: scale(1.12);
        }
        100% {
            transform: scale(1);
        }
    }
</style>
