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
    // them itself before the keymap sees them.
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
    //
    // # Over an import (plan 32 Phase 4)
    //
    // The same overlay runs over the transactions an import WOULD write when it
    // is handed `staged` (D23, D30): the cards are the import's, a save goes to
    // the import's sink as overrides and never to the network, and closing
    // keeps every edit for the write. Similar shows two groups (D24), "in this
    // import" (→ overrides) and "already in the journal" (→ edits the write
    // makes), each with its own count and check-all. A CSV import's card whose
    // description or account changed also offers "Remember as a rule" (D25),
    // whose rule is saved BEFORE the card, so the dry run that follows sees both.
    // There is no full editor for a transaction that is not in a file yet.
    //
    // # One transaction (plan 32 D31–D40)
    //
    // `kind` "edit" or "add" is SINGLE mode: the Journal's transaction editor.
    // The same card on its own, opened like a dialog, with no queue: no
    // position, no peeks, no Skip or Back. The details row is always open.
    // The footer saves (not optimistic: the card waits, then flashes a ✓ and
    // closes), with a Mark reviewed checkbox that shows the `reviewed:` tag
    // the save will write. An edit can be deleted (⋯, ⇧X, a two-step confirm
    // in the footer). An add card starts blank, with focus in the
    // description; picking a payee asks for its suggestions, and a recurring
    // charge's chip fills the amount and the funding account too. Similar
    // and Remember as a rule apply to an edit as on a queue card, and are
    // hidden on an add card, which is no transaction yet.
    //
    // # Rules from the Journal (plan 32 D27)
    //
    // A journal card offers "Remember as a rule" too, when the engine finds a
    // rules file the transaction could have come from. The choice is asked for
    // in the background as each card appears (aborted if it leaves first) and
    // kept per card for the run. The file line above Match preselects the
    // engine's recommendation; when the engine can't decide (`needsChoice`)
    // nothing is selected, and no rule is saved until the user picks. The save
    // order is the import's: the rule, then the card (the usual batch).
    import {flushSync, tick, untrack} from "svelte";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {addComment, cardChange, cardOps, formToBody, isSplit, reviewFormOf, validateForm} from "#lib/api/editMapping.js";
    import {openAutoRule} from "#lib/autoRules/openRule.js";
    import {txnModal} from "#lib/journal/edit/modalState.svelte.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {keymap, registerKeys} from "#lib/keys/keymap.svelte.js";
    import {isTypingTarget, type TargetLike} from "#lib/keys/target.js";
    import {PRIORITY, type Binding} from "#lib/keys/types.js";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import type {RuleChoice, RuleSource, RuleSpec} from "#lib/imports/reviewTypes.js";
    import {review, type ReviewCard, type SingleMode, type SingleOutcome, type StagedKind} from "#lib/stores/review.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {nextStatus} from "#lib/journal/rowModel.js";
    import {
        acceptAddChip,
        acceptChip,
        addChips,
        addDirty,
        blankItem,
        blankReviewForm,
        changeSummary,
        chipsFor,
        cycleChip,
        detailsDirty,
        footerState,
        hasUnknown,
        isDirty,
        reviewedBox,
        similarRows,
        toggleReviewed,
        type Chips,
    } from "../card";
    import type {Transaction} from "#lib/domain/types.js";
    import EditorMatchStrip from "#lib/matching/ui/EditorMatchStrip.svelte";
    import {loadRuleChoice} from "../journalRules";
    import type {QueueKind, ReviewForm} from "../types";
    import {reviewLaunch, type StagedLaunch} from "./launch.svelte";
    import {
        backdrop,
        barIn,
        cardIn,
        cardOut,
        dialogIn,
        DURATIONS,
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
    import ReviewDetails from "./ReviewDetails.svelte";
    import ReviewFields from "./ReviewFields.svelte";
    import ReviewFooter from "./ReviewFooter.svelte";
    import ReviewHero from "./ReviewHero.svelte";
    import ReviewIcon from "./ReviewIcon.svelte";
    import ReviewSettings from "./ReviewSettings.svelte";
    import RuleBuilder from "./RuleBuilder.svelte";
    import RuleFilePicker from "./RuleFilePicker.svelte";
    import SimilarSection from "./SimilarSection.svelte";
    import SingleFooter from "./SingleFooter.svelte";

    let {
        kind,
        staged = null,
        index = null,
    }: {
        /** A journal queue, an import's run, or single mode: "edit" one transaction, "add" a new one (D31, D36). */
        kind: QueueKind | StagedKind | SingleMode;
        /** An import's transactions to review before they are written. Null: the journal's queue `kind`. */
        staged?: StagedLaunch | null;
        /** The transaction an "edit" opens on. */
        index?: number | null;
    } = $props();

    const TITLES: Record<QueueKind | StagedKind | SingleMode, string> = {
        unknowns: "Review Unknowns",
        recents: "Review Recents",
        all: "Review All",
        edit: "Edit transaction",
        add: "Add transaction",
    };
    /** Fixed per mount: the page mounts the overlay for one run and unmounts it when it closes. */
    const source = untrack(() => staged);
    /** Single mode (D31), fixed per mount too. */
    const single: SingleMode | null = untrack(() => (kind === "edit" || kind === "add" ? kind : null));
    const adding = single === "add";
    const writeLabel = source?.writeLabel ?? "Write changes";
    const NO_CHIPS: Chips = {chips: [], target: null, initial: null, mode: "fix"};
    /**
     * A card's surface in the deck. Every card sits in the one grid cell, pinned
     * at its bottom edge, so the peeks' shrink-and-drop (motion.ts `PEEKS`)
     * leaves a strip of each showing below the card above it.
     */
    const SURFACE = "relative min-w-0 origin-bottom overflow-hidden rounded-box border bg-base-100 [grid-area:1/1]";
    /** The top card. Its body scrolls inside it, under a max height that leaves room for the bar and the peeks' strips. */
    const CARD = `deck-card ${SURFACE} pointer-events-auto z-[5] flex max-h-[calc(100dvh-7.5rem-var(--ll-top-inset))] flex-col self-start shadow-2xl outline-none`;
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
    type SimilarHandle = {toggleAll: () => void; setAll: (on: boolean) => void; allChecked: () => boolean};
    let similarSection = $state<SimilarHandle | null>(null);
    let stagedSection = $state<SimilarHandle | null>(null);
    let ruleBuilder = $state<{currentRule: () => RuleSpec | null} | null>(null);
    let form = $state<ReviewForm>({date: today, date2: "", status: "unmarked", code: "", description: "", comment: "", tags: [], postings: []});
    let selected = $state<number | null>(null);
    let similarOpen = $state(false);
    let checked = $state<number[]>([]);
    /** A staged run's "in this import" group (D24): its own open state and checked list (ordinals + 1, not tindices). */
    let stagedOpen = $state(false);
    let checkedStaged = $state<number[]>([]);
    /** Remember as a rule (D25), per card. */
    let ruleOn = $state(false);
    /**
     * A journal card's rules files (D27), once its load lands: null while
     * loading, and for good when there are none or the request failed.
     */
    let ruleChoice = $state<RuleChoice | null>(null);
    /** The file the user picked on this card; null takes the recommendation. */
    let rulePick = $state<string | null>(null);
    /** Every journal card's answer this run, by index (null: no toggle). An aborted load leaves no entry, so coming back asks again. */
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- a cache the load effect reads untracked on purpose: a landing load must not re-run (and abort) itself
    const ruleChoices = new Map<number, RuleChoice | null>();
    let ruleError = $state<string | null>(null);
    /**
     * Why the import failed to regenerate after a rule was saved. Shown on top
     * of the deck until dismissed: the dry-run panel that would otherwise say
     * so is underneath the backdrop, blurred, where nobody can read it.
     */
    let importNotice = $state<string | null>(null);
    let ruleSaving = $state(false);
    /** The details row (D33) opened on this card. It also stays open while one of its fields is changed. */
    let detailsOpen = $state(false);
    /** The secondary date's field is up: the card has one, or "+ secondary date" asked for it. */
    let date2Shown = $state(false);
    let confirmClose = $state(false);
    /** Single mode: ⋯ → Delete (or ⇧X) armed the delete; the footer asks to confirm it (D37). */
    let confirmDelete = $state(false);
    /** Single mode: the save landed and the ✓ is showing; the layer closes next. Keys are off. */
    let finishing = $state(false);
    /** A save was tried: the form's problems show, and follow the edits. */
    let showErrors = $state(false);
    /** The form an add card opened with, which it is dirty against. */
    let addBlank = $state.raw(blankReviewForm(today, "$"));
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
    const original = $derived(txn === null ? null : adding ? addBlank : reviewFormOf(txn));
    const chips = $derived(
        current === null || original === null ? NO_CHIPS : adding ? addChips(current.item, classes) : chipsFor(current.item, original, classes)
    );
    const dirty = $derived(txn !== null && (adding ? addDirty(addBlank, form) : isDirty(txn, form)));
    const unknown = $derived(txn !== null && hasUnknown(txn, form, classes));
    const phase = $derived(footerState(dirty, unknown));
    const split = $derived(txn !== null && !adding && isSplit(txn, form));
    const detailsChanged = $derived(original !== null && detailsDirty(original, form));
    /** Single mode shows the details row always (D33). */
    const detailsExpanded = $derived(single !== null || detailsOpen || detailsChanged);
    const errors = $derived(showErrors ? validateForm(form) : []);
    /** D32: the Mark reviewed checkbox, read from the form's tags. */
    const box = $derived(reviewedBox(form));
    /** The hero of an add card follows the description as it is typed. */
    const heroTxn = $derived(txn !== null && adding ? {...txn, description: form.description, date: form.date} : txn);
    /** What a commodity field completes from: the journal's main commodity, the card's own, then every other the journal posts. */
    const commodities = $derived([
        journal.dominantCommodity,
        ...(original?.postings.map((posting) => posting.commodity) ?? []),
        ...journal.reportStyles.keys(),
    ]);
    const change = $derived(txn === null || adding ? null : cardChange(txn, form));
    const similarSet = $derived(txn === null ? null : review.similar(txn.index));
    const rows = $derived(similarSet === null || split ? [] : similarRows(similarSet.candidates, similarSet.inQueue, change));
    /** D24's "in this import": the run's other cards with the same key. They are all in the run, so none is marked "in this run". */
    const stagedRows = $derived(source === null || txn === null || split ? [] : similarRows(review.stagedSimilar(txn.index), [], change));
    const cardIndex = $derived(txn?.index ?? null);
    /** D27: the file a journal card's rule goes to — the user's pick, else the engine's recommendation. Null: still to be chosen. */
    const rulePicked = $derived(rulePick ?? ruleChoice?.recommended ?? null);
    const journalRules = $derived(ruleChoice === null || rulePicked === null ? null : ruleChoice.source(rulePicked));
    /** Where a saved rule goes: the import's rules file, or the journal card's pick. */
    const ruleSource = $derived<RuleSource | null>(source !== null ? (source.rules ?? null) : journalRules);
    /**
     * Remember as a rule is offered on a card whose description or account
     * changed (`change` is null for a split): a CSV import's (D25), or a
     * journal card with rules files to choose from (D27).
     */
    const ruleOffered = $derived(current !== null && change !== null && (source !== null ? source.rules != null : ruleChoice !== null));
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
    const peeks = $derived(current === null || single !== null ? [] : review.cards.slice(review.cursor + 1, review.cursor + 1 + PEEK_DEPTH));

    // One run per mount: the page mounts this when a run opens and unmounts it when it closes.
    untrack(() => {
        if (source !== null) review.openStaged(source.kind, source.items, source.sink);
        else if (single === "add") review.openAdd(blankItem(today));
        else if (single === "edit") {
            if (index === null) review.close();
            else void review.openEdit(index);
        } else if (kind === "recents") void review.open(kind, {limit: settings.reviewRecentLimit, maxDays: settings.reviewMaxDays});
        else if (kind === "unknowns") void review.open(kind, {maxDays: settings.reviewMaxDays});
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
            // An add card starts in the description (D36); every other card in card mode.
            if (!sameCard) void (adding ? focusDescription() : focusCard());
        });
    });

    // D27: each journal card's rules files, asked for as it appears and
    // aborted if it leaves first. Kept for the run, a failed request as "none".
    $effect(() => {
        const index = source === null && !adding ? cardIndex : null;
        if (index === null) return;
        const known = ruleChoices.get(index);
        ruleChoice = known ?? null;
        if (known !== undefined) return;
        const controller = new AbortController();
        const settleWith = (found: RuleChoice | null): void => {
            if (controller.signal.aborted) return;
            ruleChoices.set(index, found);
            if (review.current?.item.txn.index === index) ruleChoice = found;
        };
        loadRuleChoice(index, controller.signal).then(settleWith, () => settleWith(null));
        return () => controller.abort();
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
        if (adding) addBlank = blankReviewForm(today, journal.dominantCommodity);
        const start = adding ? structuredClone(addBlank) : reviewFormOf(card.item.txn);
        form = card.draft === null ? start : structuredClone($state.snapshot(card.draft) as ReviewForm);
        selected = adding ? addChips(card.item, classes).initial : chipsFor(card.item, start, classes).initial;
        similarOpen = false;
        checked = [];
        stagedOpen = false;
        checkedStaged = [];
        ruleOn = false;
        rulePick = null;
        ruleError = null;
        detailsOpen = false;
        date2Shown = form.date2 !== "";
        confirmClose = false;
        confirmDelete = false;
        showErrors = false;
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

    async function focusDescription(): Promise<void> {
        await tick();
        focusField("description");
    }

    /** Focus field `name`: the control marked `data-review-focus` in it (an amount, not its commodity), else its first. */
    function focusField(name: string): void {
        const field = (currentCard() ?? root)?.querySelector<HTMLElement>(`[data-review-field="${name}"]`);
        (field?.querySelector<HTMLElement>("[data-review-focus]") ?? field?.querySelector<HTMLElement>("input, textarea"))?.focus();
    }

    /** Remember the card's edits as it is left (D5: they survive ← and →). */
    function stash(): void {
        if (current !== null) review.setDraft(dirty ? ($state.snapshot(form) as ReviewForm) : null);
    }

    /** The top card's arrival: a dialog's in single mode (D31), else the deck's. */
    function enter(node: Element, params: MotionParams) {
        return single !== null ? dialogIn(node, {reduced}) : cardIn(node, params);
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
        if (current.status === "saving" || ruleSaving || finishing) return;
        if (single !== null ? !dirty : phase === "clean-unknown") return;
        if (!valid()) return;
        if (ruleOffered && ruleOn) return void saveWithRule();
        commit();
    }

    /** Whether the form can be saved; once a save is tried, its problems show (and follow the edits). */
    function valid(): boolean {
        showErrors = true;
        return validateForm(form).length === 0;
    }

    /** Save the card: into the run (a queue or an import), or, in single mode, write it now. */
    function commit(): void {
        if (single !== null) void saveSingle();
        else save();
    }

    /**
     * D25: "Remember as a rule" is on. The rule is written FIRST — if it is
     * refused the card stays, edits and all, with the reason under the rule —
     * then the card is saved as usual (into the import, or the journal's
     * batch), then an import is regenerated: the card's override replays over
     * what the rule now writes. A journal card's refresh does nothing (D27).
     */
    async function saveWithRule(): Promise<void> {
        const rules = ruleSource;
        const index = txn?.index ?? null;
        if (index === null) return;
        if (rules === null) {
            ruleError = "choose a rules file for it first, or turn Remember as a rule off.";
            return;
        }
        const rule = ruleBuilder?.currentRule() ?? null;
        if (rule === null) {
            ruleError = "give it a pattern and something to set, or turn it off.";
            return;
        }
        ruleError = null;
        ruleSaving = true;
        const result = await rules.save(rule);
        ruleSaving = false;
        if (!result.ok) {
            ruleError = result.failure.message;
            return;
        }
        // Nothing can move the card while the rule saves (the keys are off),
        // but a run closed underneath has no card left to save.
        if (review.current?.item.txn.index === index) commit();
        importNotice = await rules.refresh();
    }

    /** Save the card on screen, with the checked similar rows, and advance (D14). */
    function save(): void {
        if (current === null || txn === null) return;
        const reviewedOn = phase === "dirty-unknown" ? null : today;
        const ops = cardOps(txn, form, {key: current.item.key, reviewedOn, defaultCommodity: journal.dominantCommodity});
        if (ops.length === 0) return;
        const picked = new Set(checked);
        const pickedStaged = new Set(checkedStaged);
        const similar =
            change === null
                ? []
                : [
                      ...stagedRows.filter((row) => pickedStaged.has(row.txn.txn.index)).map((row) => ({txn: row.txn, ops: row.ops})),
                      ...rows.filter((row) => picked.has(row.txn.txn.index)).map((row) => ({txn: row.txn, ops: row.ops})),
                  ];
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

    /**
     * Single mode's save (D31, D32, D36): an edit as one batch with its
     * checked similar rows, or an add as one POST. The `reviewed:` tag is
     * whatever the checkbox left in the form's tags, so nothing is stamped
     * here, and similar rows never are (D38). A save that lands flashes a ✓
     * and closes, the journal's cursor going to the transaction. A conflict
     * closes too: the page's banner says the journal changed.
     */
    async function saveSingle(): Promise<void> {
        if (current === null || txn === null) return;
        const commodity = journal.dominantCommodity;
        let outcome: SingleOutcome;
        if (adding) {
            const {date, date2, status, code, description, postings} = form;
            outcome = await review.addSingle(formToBody({date, date2, status, code, description, comment: addComment(form), postings}, commodity));
        } else {
            const ops = cardOps(txn, form, {key: current.item.key, reviewedOn: null, defaultCommodity: commodity});
            const picked = new Set(checked);
            const similar = change === null ? [] : rows.filter((row) => picked.has(row.txn.txn.index)).map((row) => ({txn: row.txn, ops: row.ops}));
            outcome = await review.saveSingle(ops, similar);
        }
        if (outcome.ok) return finish(outcome.index);
        if (outcome.failure.kind === "conflict") close();
    }

    /** The save landed: a short ✓ on the card, then close, with the journal's cursor going to `index`. */
    function finish(index: number | null): void {
        const shown = txn?.index ?? null;
        lastSaved = index;
        finishing = true;
        flushSync(() => (stamped = shown));
        setTimeout(close, DURATIONS.flash);
    }

    /** `v` (D32): toggle Mark reviewed. Not while a posting is unknown. */
    function toggleReviewedBox(): void {
        if (original === null || unknown || current?.status === "saving") return;
        form.tags = toggleReviewed(form, original, today);
    }

    /** ⋯ → Delete transaction, `⇧X` (D37): arm it; asked again (the button, or ⇧X), delete. */
    function armDelete(): void {
        if (single !== "edit" || current === null || current.status === "saving") return;
        menuOpen = false;
        if (confirmDelete) return void deleteTxn();
        confirmDelete = true;
    }

    /** Delete the transaction, then close. The journal's cursor stays where it is, as after the table's `x`. */
    async function deleteTxn(): Promise<void> {
        confirmDelete = false;
        const outcome = await review.deleteSingle();
        if (outcome.ok || outcome.failure.kind === "conflict") {
            lastSaved = null;
            close();
        }
    }

    /** An add card's payee was picked (D36): ask for its suggestions, and select the strong one. */
    async function pickPayee(payee: string): Promise<void> {
        await review.suggestFor(payee);
        selected = chips.initial;
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
        if (adding) {
            const filled = acceptAddChip(form, chips.target, chip);
            filled.postings.forEach((row, at) => form.postings[at] !== undefined && Object.assign(form.postings[at], row));
            form.description = filled.description;
            selected = at;
            return;
        }
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

    function openRule(rule: number): void {
        close();
        void openAutoRule(rule);
    }

    /** `s`: open every Similar group on the card (both, in a staged run), or close them all if they are open. */
    async function toggleSimilar(): Promise<void> {
        if (rows.length === 0 && stagedRows.length === 0) return;
        const opening = !((rows.length === 0 || similarOpen) && (stagedRows.length === 0 || stagedOpen));
        similarOpen = opening && rows.length > 0;
        stagedOpen = opening && stagedRows.length > 0;
        if (!opening) return void focusCard();
        await tick();
        (currentCard() ?? root)?.querySelector<HTMLInputElement>("[data-similar-rows] input:not([disabled])")?.focus();
    }

    /** ⇧A: check every similar row on the card — both groups in a staged run — or, when all are checked, none. */
    function toggleAllSimilar(): void {
        const sections = [similarSection, stagedSection].filter((section): section is SimilarHandle => section !== null);
        const on = !sections.every((section) => section.allChecked());
        for (const section of sections) section.setAll(on);
    }

    /** `R` (D25): turn "Remember as a rule" on or off. Turned on with its file still to be chosen (D27), the picker takes focus. */
    async function toggleRule(): Promise<void> {
        if (!ruleOffered || ruleSaving) return;
        ruleOn = !ruleOn;
        ruleError = null;
        if (!ruleOn || ruleSource !== null) return;
        await tick();
        (currentCard() ?? root)?.querySelector<HTMLSelectElement>("[data-testid='review-rule-file-select']")?.focus();
    }

    /** D27: the user chose the file a journal card's rule goes to. */
    function pickRuleFile(rulesId: string): void {
        rulePick = rulesId;
        ruleError = null;
    }

    /** `D`, `#`, a click on the details line: open the details row (D33) and put the caret in `field`. */
    async function openDetails(field: "date" | "code" | "date2"): Promise<void> {
        if (current === null) return;
        detailsOpen = true;
        await tick();
        focusField(field);
    }

    /** `!` (D33): the next status in the Journal's cycle, unmarked → pending → cleared. The row opens, so the change shows. */
    function cycleStatus(): void {
        if (current === null || current.status === "saving") return;
        detailsOpen = true;
        form.status = nextStatus(form.status);
    }

    /** ⋯ → Add secondary date, or the row's "+ secondary date". */
    function addDate2(): void {
        menuOpen = false;
        date2Shown = true;
        void openDetails("date2");
    }

    function removeDate2(): void {
        form.date2 = "";
        date2Shown = false;
    }

    /** Fold the details back to their line; only offered while none of them is changed. */
    function collapseDetails(): void {
        detailsOpen = false;
        date2Shown = form.date2 !== "";
        void focusCard();
    }

    function editAccount(at: number | null): void {
        const row = at ?? Math.max(0, chips.mode === "fix" && chips.target !== null ? chips.target : 0);
        if (row < form.postings.length) focusField(`account-${row}`);
    }

    function close(): void {
        confirmClose = false;
        confirmDelete = false;
        // The layer fades and drops as one; the cards do nothing of their own,
        // and the outgoing one stays in the grid's flow (see the styles).
        finishMotion(stack);
        motion.exit = "close";
        root?.setAttribute("data-closing", "");
        review.close();
        reviewLaunch.close(lastSaved);
        if (single !== null) txnModal.close();
    }

    /** Escape in card mode, ✕, the backdrop: close — but ask first if the card has unsaved edits. */
    function requestClose(): void {
        if (finishing) return;
        if (confirmDelete) {
            confirmDelete = false;
            return void focusCard();
        }
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

    const live = (): boolean => !keymap.helpOpen && !confirmClose && !confirmDelete && !ruleSaving && !finishing;
    const onCard = (): boolean => live() && review.current !== null;
    /** Keys that walk a queue (D16), or that only make sense in one (Reviewed-and-next, Ignore). Single mode has none of them (D40). */
    const queueKeys: Binding[] =
        single !== null
            ? []
            : [
                  {keys: "y", label: "Reviewed (or save the changes)", group: "Review", enabled: live, run: primary},
                  {keys: "Y", label: "Accept the suggestion, mark reviewed, next", group: "Review", enabled: onCard, run: accept},
                  {keys: "ArrowRight", label: "Skip", group: "Review", enabled: onCard, run: skip},
                  {keys: "n", label: "Skip", group: "Review", enabled: onCard, run: skip},
                  {keys: "l", label: "Skip", group: "Review", enabled: onCard, run: skip},
                  {keys: "ArrowLeft", label: "Previous card", group: "Review", enabled: live, run: back},
                  {keys: "p", label: "Previous card", group: "Review", enabled: live, run: back},
                  {keys: "h", label: "Previous card", group: "Review", enabled: live, run: back},
                  {keys: "i", label: "Ignore (never ask again)", group: "Review", enabled: onCard, run: ignore},
              ];
    /** Single mode's own keys (D40): Mark reviewed, and delete on an edit. */
    const singleKeys: Binding[] =
        single === null
            ? []
            : [
                  {keys: "v", label: "Mark reviewed, or not", group: "Review", enabled: onCard, run: toggleReviewedBox},
                  ...(single === "edit"
                      ? [
                            {
                                keys: "X",
                                label: "Delete the transaction (press again to confirm)",
                                group: "Review" as const,
                                enabled: () => !keymap.helpOpen && !confirmClose && !finishing && review.current !== null,
                                run: armDelete,
                            },
                        ]
                      : []),
              ];
    const closeLabel = single === null ? "Close review" : "Close the editor";
    const saveLabel = single === null ? "Reviewed (or save the changes)" : adding ? "Add the transaction" : "Save";
    const acceptLabel =
        single === null ? "Accept the suggestion, mark reviewed, next" : adding ? "Accept the suggestion and add" : "Accept the suggestion and save";

    registerKeys({
        id: "review-overlay",
        priority: PRIORITY.overlay,
        modal: true,
        bindings: [
            {keys: "mod+Enter", label: saveLabel, group: "Review", enabled: live, run: primary},
            {keys: "mod+shift+Enter", label: acceptLabel, group: "Review", enabled: onCard, run: accept},
            {keys: "Enter", label: saveLabel, group: "Review", enabled: live, run: () => activateFocused() || primary()},
            ...queueKeys,
            ...singleKeys,
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
            {keys: "D", label: "Edit the date (and the other details)", group: "Review", enabled: onCard, run: () => void openDetails("date")},
            {keys: "!", label: "Cycle the status: unmarked, pending, cleared", group: "Review", enabled: onCard, run: cycleStatus},
            {keys: "#", label: "Edit the code", group: "Review", enabled: onCard, run: () => void openDetails("code")},
            {keys: "t", label: "Add a tag", group: "Review", enabled: onCard, run: () => focusField("tags")},
            {keys: "m", label: "Edit comment", group: "Review", enabled: onCard, run: () => focusField("comment")},
            {keys: "s", label: "Open or close Similar", group: "Review", enabled: onCard, run: () => void toggleSimilar()},
            {
                keys: " ",
                label: "Toggle the focused similar row",
                group: "Review",
                enabled: () => onCard() && (similarOpen || stagedOpen),
                run: () => void activateFocused(),
            },
            {keys: "A", label: "Check or uncheck all similar", group: "Review", enabled: onCard, run: toggleAllSimilar},
            {keys: "R", label: "Remember as a rule", group: "Review", enabled: () => onCard() && ruleOffered, run: () => void toggleRule()},
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
        <!-- Top bar: queue, position, progress, saving, ⋯, ✕. `backdrop-blur` makes
             it a stacking context, so it needs a z-index above the deck's cards
             (z-5/z-6) or the ⋯ menu opens beneath the card. -->
        <div
            class="@container pointer-events-auto relative z-20 flex w-full max-w-4xl shrink-0 flex-wrap items-center gap-x-3 gap-y-1 rounded-box border border-base-content/10 bg-base-300/95 py-1.5 pr-1.5 pl-4 text-base-content shadow-lg backdrop-blur"
            in:barIn|global={{reduced}}
        >
            <h2 class="min-w-0 truncate text-sm font-semibold tracking-wide">{TITLES[kind]}</h2>
            {#if review.count > 0 && single === null}
                <span class="shrink-0 text-sm whitespace-nowrap text-base-content/65 tabular-nums" data-testid="review-position" aria-live="polite">
                    {review.position} of {review.count}
                </span>
                <div
                    class="h-1 w-28 min-w-6 shrink overflow-hidden rounded-full bg-base-content/15 @max-[24rem]:hidden"
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
            {#if review.saving > 0 && single === null}
                <span class="flex items-center gap-1.5 text-xs text-base-content/60" data-testid="review-saving" role="status">
                    <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-success"></span> saving…
                </span>
            {/if}
            <!-- Together and last: a wrapped bar keeps ⋯ and ✕ on the right of its second line. -->
            <div class="ml-auto flex shrink-0 items-center gap-1">
                <details
                    class="dropdown dropdown-end"
                    bind:open={menuOpen}
                    use:dismissible={{active: menuOpen, outside: true, onDismiss: () => (menuOpen = false)}}
                >
                    <summary
                        class="btn btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10"
                        aria-label="More actions"
                        title="More actions"
                    >
                        <ReviewIcon name="ellipsis" />
                    </summary>
                    <div class="dropdown-content z-30 mt-1 rounded-box border border-base-300 bg-base-200 p-2 text-base-content shadow-xl">
                        {#if settingsOpen}
                            <ReviewSettings show="review" />
                            <button type="button" class="btn mt-2 w-full btn-ghost btn-xs" onclick={() => (settingsOpen = false)}>Done</button>
                        {:else}
                            <ul class="menu w-60 max-w-[calc(100vw-3rem)] p-0 text-sm">
                                {#if single === null}
                                    <!-- Single mode has its Mark reviewed checkbox instead (D32). -->
                                    <li>
                                        <button type="button" onclick={ignore} disabled={current === null}>
                                            <ReviewIcon name="ban" /> Ignore — never ask again <kbd class="ml-auto kbd kbd-xs">i</kbd>
                                        </button>
                                    </li>
                                {/if}
                                {#if form.date2 === "" && !date2Shown}
                                    <li>
                                        <button type="button" onclick={addDate2} disabled={current === null}>
                                            <ReviewIcon name="calendar" /> Add secondary date
                                        </button>
                                    </li>
                                {/if}
                                {#if single === "edit"}
                                    <!-- D37: only here, never in a queue run, where a missed key costs too much. -->
                                    <li>
                                        <button type="button" class="text-error" onclick={armDelete} disabled={current === null} data-testid="review-delete">
                                            <ReviewIcon name="x" /> Delete transaction <kbd class="ml-auto kbd kbd-xs">⇧X</kbd>
                                        </button>
                                    </li>
                                {/if}
                                {#if source === null && single === null}
                                    <!-- The window and the limit shape the journal's queues; an import's run is the whole import. -->
                                    <li>
                                        <button type="button" onclick={() => (settingsOpen = true)}><ReviewIcon name="sliders" /> Settings</button>
                                    </li>
                                {/if}
                            </ul>
                        {/if}
                    </div>
                </details>
                <button
                    type="button"
                    class="btn btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10"
                    aria-label={closeLabel}
                    title={hint("Close", "Esc")}
                    onclick={requestClose}
                >
                    <ReviewIcon name="x" />
                </button>
            </div>
        </div>

        {#if importNotice !== null}
            <div class="pointer-events-auto alert w-full max-w-4xl alert-error shadow-lg" role="alert" data-testid="review-import-notice">
                <div class="min-w-0 grow">
                    <p class="font-medium">The rule was saved, but the import couldn't be regenerated</p>
                    <pre class="mt-1 max-h-32 overflow-auto text-xs break-words whitespace-pre-wrap">{importNotice}</pre>
                </div>
                <button type="button" class="btn shrink-0 btn-ghost btn-sm" onclick={() => (importNotice = null)}>Dismiss</button>
            </div>
        {/if}
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
                    <span class="loading loading-sm loading-spinner"></span>
                    {single !== null ? "Loading the transaction…" : "Loading the queue…"}
                </div>
            {:else if review.error !== null}
                <div class="{PANEL} p-5 sm:p-8" role="alert">
                    <div class="alert alert-error">
                        <span>{single !== null ? "Couldn't load the transaction" : "Couldn't load the queue"}: {review.error}</span>
                    </div>
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
                        {#if source !== null}
                            {kind === "unknowns" ? "Every transaction in this import has a category." : "This import has nothing to write."}
                        {:else}
                            {kind === "unknowns" ? "Every transaction in the window has a category." : "Everything in the window is reviewed."}
                        {/if}
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
                        {#if source !== null}
                            <p class="max-w-sm text-sm text-base-content/60" data-testid="review-staged-note">
                                Nothing is written yet. These edits are applied when you press <strong class="font-medium text-base-content/80"
                                    >{writeLabel}</strong
                                >.
                            </p>
                        {/if}
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
                            in:enter|global={motion}
                            out:cardOut|global={motion}
                        >
                            <!-- The card's body scrolls inside the card, so the deck never clips it.
                                 Up and down only: nothing in it may be wider than the card
                                 (plan 42), and `overflow-x-hidden` stops a stray pixel from
                                 turning it into a canvas that pans sideways under a finger. -->
                            <div class="flex min-h-0 min-w-0 flex-col gap-6 overflow-x-hidden overflow-y-auto overscroll-contain p-5 sm:p-8">
                                <ReviewHero txn={heroTxn ?? txn} {classes} {updatedFrom} detailsBlock={detailsExpanded}>
                                    {#snippet details()}
                                        <ReviewDetails
                                            bind:form
                                            mode={detailsExpanded ? "editor" : "summary"}
                                            showDate2={date2Shown || form.date2 !== ""}
                                            canCollapse={single === null && !detailsChanged}
                                            disabled={current.status === "saving"}
                                            onExpand={() => void openDetails("date")}
                                            onCollapse={collapseDetails}
                                            onAddDate2={addDate2}
                                            onRemoveDate2={removeDate2}
                                        />
                                    {/snippet}
                                </ReviewHero>
                                <div class="border-t border-dashed border-base-content/15"></div>
                                {#if single === "edit"}
                                    <!-- Plan 35 A2: ⇄ Find match, or the transaction's links with Unlink. -->
                                    <EditorMatchStrip
                                        {txn}
                                        {dirty}
                                        disabled={current.status === "saving" || finishing}
                                        onUnlinked={() => txnModal.openEdit(txn.index)}
                                    />
                                {/if}
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
                                    {commodities}
                                    defaultCommodity={journal.dominantCommodity}
                                    disabled={current.status === "saving"}
                                    onTabChip={tabChip}
                                    onPickChip={pick}
                                    onRule={openRule}
                                    onPayeePick={adding ? (payee) => void pickPayee(payee) : undefined}
                                />
                                {#if errors.length > 0}
                                    <ul class="-mt-2 list-inside list-disc text-sm text-error" role="alert" data-testid="review-errors">
                                        {#each errors as message (message)}
                                            <li>{message}</li>
                                        {/each}
                                    </ul>
                                {/if}
                                {#if split}
                                    <p class="text-xs text-base-content/50" data-testid="review-split-note">Splits aren't applied to similar transactions.</p>
                                {:else if source !== null}
                                    <!-- D24: two sources, two groups, each with its own count and check-all. -->
                                    {#if stagedRows.length > 0}
                                        <SimilarSection
                                            bind:this={stagedSection}
                                            rows={stagedRows}
                                            bind:checked={checkedStaged}
                                            bind:open={stagedOpen}
                                            applying={changeSummary(change)}
                                            title="Similar in this import"
                                            testid="review-similar-staged"
                                        />
                                    {/if}
                                    {#if rows.length > 0}
                                        <SimilarSection
                                            bind:this={similarSection}
                                            {rows}
                                            bind:checked
                                            bind:open={similarOpen}
                                            applying={changeSummary(change)}
                                            title="Similar already in the journal"
                                            testid="review-similar-journal"
                                        />
                                    {/if}
                                {:else if rows.length > 0}
                                    <SimilarSection bind:this={similarSection} {rows} bind:checked bind:open={similarOpen} applying={changeSummary(change)} />
                                {/if}
                                {#if ruleOffered && change !== null}
                                    <RuleBuilder
                                        bind:this={ruleBuilder}
                                        bind:on={ruleOn}
                                        source={ruleSource}
                                        item={current.item}
                                        original={txn}
                                        {change}
                                        error={ruleError}
                                        busy={ruleSaving}
                                        file={source === null && ruleChoice !== null ? fileLine : undefined}
                                    />
                                    {#snippet fileLine()}
                                        {#if ruleChoice !== null}
                                            <RuleFilePicker choice={ruleChoice} picked={rulePicked} onPick={pickRuleFile} disabled={ruleSaving} />
                                        {/if}
                                    {/snippet}
                                {/if}
                            </div>

                            {#if confirmClose}
                                <div
                                    class="flex min-h-[4.5rem] shrink-0 flex-wrap items-center justify-between gap-3 border-t border-warning/30 bg-warning/10 px-5 py-3 text-sm sm:px-8"
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
                            {:else if confirmDelete}
                                <div
                                    class="flex min-h-[4.5rem] shrink-0 flex-wrap items-center justify-between gap-3 border-t border-error/30 bg-error/10 px-5 py-3 text-sm sm:px-8"
                                    role="alert"
                                    data-testid="review-confirm-delete"
                                >
                                    <span>Delete this transaction from the journal?</span>
                                    <span class="flex gap-2">
                                        <button type="button" class="btn btn-error btn-sm" onclick={() => void deleteTxn()}
                                            >Confirm delete <kbd class="kbd kbd-xs">⇧X</kbd></button
                                        >
                                        <button type="button" class="btn btn-ghost btn-sm" onclick={() => ((confirmDelete = false), void focusCard())}
                                            >Keep</button
                                        >
                                    </span>
                                </div>
                            {:else if single !== null}
                                <SingleFooter
                                    {adding}
                                    {dirty}
                                    busy={current.status === "saving" || ruleSaving || finishing}
                                    reviewed={box.checked}
                                    reviewedLabel={box.label}
                                    {unknown}
                                    onSave={primary}
                                    onToggleReviewed={toggleReviewedBox}
                                    onRevert={revert}
                                />
                            {:else}
                                <ReviewFooter
                                    {phase}
                                    {reduced}
                                    busy={current.status === "saving" || ruleSaving}
                                    onPrimary={primary}
                                    onSkip={skip}
                                    onRevert={revert}
                                />
                            {/if}

                            {#if stamped === txn.index && single !== null}
                                <!-- Single mode (D31): a short ✓ in the middle of the card, then the layer closes. -->
                                <div
                                    class="pointer-events-none absolute inset-0 grid place-items-center bg-base-100/50"
                                    data-testid="review-flash"
                                    aria-hidden="true"
                                >
                                    <span
                                        class="grid h-16 w-16 place-items-center rounded-full bg-success text-success-content shadow-lg"
                                        in:stampPop={{reduced}}
                                    >
                                        <ReviewIcon name="check" class="h-8 w-8" />
                                    </span>
                                </div>
                            {:else if stamped === txn.index}
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
        aria-label={closeLabel}
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
