<script lang="ts">
    // Find Matches (plan 35 D9): one proposed transfer pair at a time, on the
    // review overlay's frame, keys and motion (plan 32 D16, D29).
    //
    // # Screens
    //
    // The run opens on the COUNTS ("212 clear, 9 to check") with Link all N
    // clear matches (asks first; one request, written by the engine in
    // batch-sized saves) and Review one at a time. Then one card per pair:
    // the header "⇄ Checking → Amex · $500.00", the confidence and the gap,
    // both halves side by side with editable descriptions, and an ambiguous
    // pair's alternatives as radio choices (the nearest selected). The run
    // ends on a tally.
    //
    // A suggested combination (Amendment A4) is a card too: the single posting
    // on the left, its group (2–4 postings) on the right. Link writes one id
    // on every member; it has no Not a match (rejections are pairwise), and
    // Ignore this side is for the single. It is never "clear", so never in
    // Link all. When the lookback hides older pairs (A1), the first screen
    // says how many, with Show all.
    //
    // Opened with `matches.openFor(index)`, the overlay is FOCUSED on one
    // transaction instead (A2): `MatchFocus` takes the deck and its own keys.
    //
    // # Writes
    //
    // Not optimistic: Link, Not a match and Ignore this side wait for their
    // write (`matches` store), which re-reads the journal and the proposals;
    // the card on screen is always the first proposal not skipped, so the
    // re-read is what moves the deck on. A description edited on the card is
    // saved IN the same write (Link and Save; Not a match saves it too). A 409
    // re-reads everything and says so on the card; any other failure stays
    // on the card with the edits. Skip lasts for this run only.
    //
    // # Keys
    //
    // A `PRIORITY.overlay`, `modal: true` layer, the "Matches" group of `?`.
    // Card mode / field mode as in review: single keys act on the card,
    // Escape leaves a field, then closes (asking first when edits would go).
    import {tick, untrack} from "svelte";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {keymap, registerKeys} from "#lib/keys/keymap.svelte.js";
    import {isTypingTarget, type TargetLike} from "#lib/keys/target.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {longDate, shortDate} from "#lib/review/card.js";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
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
        type CardExit,
        type MotionParams,
    } from "#lib/review/ui/motion.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import ReviewSettings from "#lib/review/ui/ReviewSettings.svelte";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {matches, type MatchOutcome} from "#lib/stores/matches.svelte.js";
    import {choicesOf, gapLabel, halfView, headerOf, olderLabel, otherHalf, pairKey, wireHalf} from "../model";
    import type {MatchHalf, MatchProposal} from "../types";
    import MatchFocus from "./MatchFocus.svelte";
    import MatchFooter from "./MatchFooter.svelte";
    import MatchHalfView from "./MatchHalf.svelte";
    import MatchLine from "./MatchLine.svelte";

    const SURFACE = "relative min-w-0 origin-bottom overflow-hidden rounded-box border bg-base-100 [grid-area:1/1]";
    const CARD = `deck-card ${SURFACE} pointer-events-auto z-[5] flex max-h-[calc(100dvh-7.5rem)] flex-col self-start shadow-2xl outline-none`;
    const PANEL = `${SURFACE} pointer-events-auto z-0 self-start border-base-content/10 shadow-2xl outline-none`;
    const PEEK = `${SURFACE} pointer-events-none self-stretch border-base-content/10 p-5 shadow-xl sm:p-8`;
    const today = localToday();
    const reduced = prefersReducedMotion();
    const motion: MotionParams = {exit: "open", reduced};

    let root = $state<HTMLDivElement | null>(null);
    let stack = $state<HTMLDivElement | null>(null);
    let focusView = $state<{onKeydown: (event: KeyboardEvent) => void; escape: () => void} | null>(null);
    /** The counts screen, then the cards. */
    let started = $state(false);
    let confirmAll = $state(false);
    /** "Linked 212 pairs." after Link all. */
    let linkedAll = $state<number | null>(null);
    /** Which choice the card's radios picked (0: the proposal itself). */
    let choiceAt = $state(0);
    /** The descriptions as edited, by transaction index; absent means as written. */
    let drafts = $state<Record<number, string>>({});
    /** Ignore this side, armed for that side and asking to confirm. */
    let confirmIgnore = $state<0 | 1 | null>(null);
    let confirmClose = $state(false);
    let failure = $state<string | null>(null);
    /** A 409 re-read everything: said on the card, not in the page's banner the overlay covers. */
    let notice = $state<string | null>(null);
    let menuOpen = $state(false);
    const tally = $state({linked: 0, rejected: 0, ignored: 0, skipped: 0});
    let seededFor: string | null = null;

    const page = $derived(matches.page);
    /** Focused on one transaction (A2): `MatchFocus` owns the deck and the keys. */
    const focused = $derived(matches.focus !== null);
    const older = $derived(olderLabel(page?.older ?? 0));
    const queue = $derived(matches.queue);
    const proposal = $derived<MatchProposal | null>(queue[0] ?? null);
    const choices = $derived(proposal === null ? [] : choicesOf(proposal));
    const choice = $derived(choices[choiceAt] ?? choices[0] ?? null);
    const header = $derived(choice === null || proposal === null ? null : headerOf(choice, proposal));
    const cardKey = $derived(proposal === null ? null : pairKey(proposal.a, proposal.b));
    const clearPairs = $derived((page?.proposals ?? []).filter((candidate) => candidate.clear));
    const checkCount = $derived((page?.proposals.length ?? 0) - clearPairs.length);
    const empty = $derived(page !== null && page.proposals.length === 0 && linkedAll === null && !started);
    /** A suggested combination (A4): one posting against a group. */
    const combination = $derived(proposal !== null && proposal.group.length > 0);
    const finished = $derived(started && proposal === null);
    const busy = $derived(matches.busy);
    const peeks = $derived(started ? queue.slice(1, 1 + PEEK_DEPTH) : []);

    /** The clearing account a half went through. */
    const halfAccount = (half: MatchHalf | undefined): string => (half === undefined ? "?" : (halfView(half).clearing?.account ?? "?"));
    const describe = (half: MatchHalf): string => drafts[half.txn.index] ?? half.txn.description;
    /** The halves whose descriptions the card edits: both of a pair, a combination's single. */
    const editable = $derived(choice === null ? [] : combination ? [choice.a] : [choice.a, choice.b]);
    const dirty = $derived(editable.some((half) => describe(half).trim() !== half.txn.description));
    const sides = $derived<readonly string[]>(editable.map((half) => `${shortDate(half.txn.date)} · ${describe(half)}`));

    // The completion list for the descriptions (D15's payees), as review loads it.
    $effect(() => {
        const generation = journal.generation;
        untrack(() => void reviewLaunch.loadVocabulary(generation));
    });

    // Each new card: the nearest choice, the descriptions as written, nothing armed.
    $effect(() => {
        const key = cardKey;
        untrack(() => {
            if (key === seededFor) return;
            seededFor = key;
            choiceAt = 0;
            drafts = {};
            confirmIgnore = null;
            confirmClose = false;
            failure = null;
            if (!focused) void focusCard();
        });
    });

    /** The card on screen (while one exits, two are in the DOM). */
    function currentCard(): HTMLElement | null {
        return cardKey === null ? null : (stack?.querySelector<HTMLElement>(`[data-card-key="${CSS.escape(cardKey)}"]`) ?? null);
    }

    async function focusCard(): Promise<void> {
        await tick();
        const target = currentCard() ?? stack?.querySelector<HTMLElement>("[data-match-panel]") ?? null;
        (target ?? root)?.focus({preventScroll: true});
    }

    function focusDescription(side: "left" | "right"): void {
        currentCard()?.querySelector<HTMLInputElement>(`[data-match-field="description-${side}"] input`)?.focus();
    }

    function exitAs(exit: CardExit): void {
        finishMotion(stack);
        motion.exit = exit;
    }

    /**
     * A write's outcome: a conflict re-read everything; another failure stays
     * on the card. Either way the descriptions typed (`kept`, as they were
     * when the write went) stay on the card for the halves still showing:
     * the re-read may have re-seeded it underneath (review's D14 rule).
     */
    function settle(outcome: MatchOutcome<unknown>, kept: Record<number, string>): boolean {
        if (outcome.ok) {
            notice = null;
            return true;
        }
        if (outcome.failure.kind === "conflict") {
            notice = "The journal changed since these pairs were found, so nothing was written. They have been read again: check this one and try again.";
        } else failure = outcome.failure.message;
        const showing = choice === null ? [] : [choice.a.txn.index, choice.b.txn.index];
        drafts = Object.fromEntries(Object.entries(kept).filter(([index]) => showing.includes(Number(index))));
        return false;
    }

    /** The pair the card would write, with the descriptions edited on it. */
    function pairBody(): {a: ReturnType<typeof wireHalf>; b: ReturnType<typeof wireHalf>} | null {
        if (choice === null) return null;
        return {a: wireHalf(choice.a, describe(choice.a)), b: wireHalf(choice.b, describe(choice.b))};
    }

    /** What Link writes: the pair, or a combination's single and its whole group (A3). */
    function linkBody(): ReturnType<typeof pairBody> | {members: ReturnType<typeof wireHalf>[]} {
        if (choice === null || proposal === null) return null;
        if (!combination) return pairBody();
        return {members: [wireHalf(choice.a, describe(choice.a)), ...proposal.group.map((half) => wireHalf(half))]};
    }

    function start(): void {
        if (page === null || page.proposals.length === 0) return close();
        confirmAll = false;
        started = true;
        motion.exit = "open";
        void focusCard();
    }

    async function linkAll(): Promise<void> {
        if (busy || clearPairs.length === 0) return;
        if (!confirmAll) {
            confirmAll = true;
            return;
        }
        confirmAll = false;
        failure = null;
        const outcome = await matches.linkAll(clearPairs);
        if (settle(outcome, {}) && outcome.ok) linkedAll = outcome.value.linked;
        void focusCard();
    }

    async function link(): Promise<void> {
        const body = linkBody();
        if (body === null || busy || confirmIgnore !== null) return;
        failure = null;
        exitAs("reviewed");
        const kept = {...drafts};
        if (settle(await matches.link(body), kept)) tally.linked += 1;
    }

    async function reject(): Promise<void> {
        const body = pairBody();
        if (body === null || busy || confirmIgnore !== null || combination) return;
        failure = null;
        exitAs("ignore");
        const kept = {...drafts};
        if (settle(await matches.reject(body), kept)) tally.rejected += 1;
    }

    /** ⋯ → Ignore this side: arm it, then confirm (it is permanent, D1). A combination has one side: its single. */
    async function ignore(side: 0 | 1): Promise<void> {
        if (choice === null || busy || (combination && side === 1)) return;
        if (confirmIgnore !== side) {
            confirmIgnore = side;
            return;
        }
        confirmIgnore = null;
        failure = null;
        const half = side === 0 ? choice.a : choice.b;
        exitAs("ignore");
        const kept = {...drafts};
        if (settle(await matches.ignore(wireHalf(half, describe(half))), kept)) tally.ignored += 1;
    }

    function skip(): void {
        if (proposal === null || busy) return;
        exitAs("skip");
        tally.skipped += 1;
        matches.skip(proposal);
    }

    function revert(): void {
        drafts = {};
        void focusCard();
    }

    /** `c`: the next alternative of an ambiguous pair. */
    function cycleChoice(): void {
        if (choices.length > 1) choiceAt = (choiceAt + 1) % choices.length;
    }

    /** The primary action of whatever is showing: start, Link, or close. */
    function primary(): void {
        if (confirmAll) return void linkAll();
        if (confirmIgnore !== null) return void ignore(confirmIgnore);
        if (!started) return start();
        if (finished || empty) return close();
        void link();
    }

    function close(): void {
        confirmClose = false;
        finishMotion(stack);
        motion.exit = "close";
        root?.setAttribute("data-closing", "");
        matches.close();
    }

    function requestClose(): void {
        if (focused) return focusView === null ? close() : focusView.escape();
        if (confirmAll || confirmIgnore !== null || confirmClose) {
            confirmAll = false;
            confirmIgnore = null;
            confirmClose = false;
            return void focusCard();
        }
        if (dirty) {
            confirmClose = true;
            return;
        }
        close();
    }

    function activateFocused(): boolean {
        const focused = document.activeElement;
        if (focused instanceof HTMLButtonElement || (focused instanceof HTMLInputElement && focused.type === "radio")) {
            if (root?.contains(focused) === true) {
                focused.click();
                return true;
            }
        }
        return false;
    }

    // Element-level, before the window keymap: Escape leaves a field, ⌘↵ links from one.
    function onKeydown(event: KeyboardEvent): void {
        finishMotion(stack);
        if (focused) return focusView?.onKeydown(event);
        if (event.defaultPrevented || event.isComposing) return;
        if (event.key === "Escape" && isTypingTarget(event.target as TargetLike | null)) {
            event.preventDefault();
            void focusCard();
            return;
        }
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.altKey) {
            event.preventDefault();
            primary();
        }
    }

    const live = (): boolean => !keymap.helpOpen && !confirmClose && !busy && !focused;
    const onCard = (): boolean => live() && started && proposal !== null && !confirmAll;

    registerKeys({
        id: "match-overlay",
        priority: PRIORITY.overlay,
        modal: true,
        bindings: [
            {keys: "mod+Enter", label: "Link (and save the descriptions)", group: "Matches", enabled: live, run: primary},
            {keys: "Enter", label: "Link, or the focused button", group: "Matches", enabled: live, run: () => activateFocused() || primary()},
            {keys: "y", label: "Link (and save the descriptions)", group: "Matches", enabled: () => onCard() && confirmIgnore === null, run: () => void link()},
            {
                keys: "a",
                label: "Link all clear matches (asks first)",
                group: "Matches",
                enabled: () => live() && !started && clearPairs.length > 0,
                run: () => void linkAll(),
            },
            {
                keys: "x",
                label: "Not a match (remember it)",
                group: "Matches",
                enabled: () => onCard() && confirmIgnore === null && !combination,
                run: () => void reject(),
            },
            {keys: "ArrowRight", label: "Skip", group: "Matches", enabled: onCard, run: skip},
            {keys: "n", label: "Skip", group: "Matches", enabled: onCard, run: skip},
            {keys: "l", label: "Skip", group: "Matches", enabled: onCard, run: skip},
            {keys: "c", label: "Choose the next alternative", group: "Matches", enabled: () => onCard() && choices.length > 1, run: cycleChoice},
            {keys: "1", label: "Edit the left description", group: "Matches", enabled: onCard, run: () => focusDescription("left")},
            {keys: "2", label: "Edit the right description", group: "Matches", enabled: () => onCard() && !combination, run: () => focusDescription("right")},
            {keys: "z", label: "Revert the descriptions", group: "Matches", enabled: () => onCard() && dirty, run: revert},
            {keys: "Escape", label: "Leave the field, or close", group: "Matches", run: requestClose},
            {keys: "?", label: "Show keyboard shortcuts", group: "Matches", run: () => keymap.toggleHelp()},
        ],
    });
</script>

{#snippet olderNote()}
    {#if older !== null && page !== null}
        <p class="flex flex-wrap items-center justify-center gap-2 text-sm text-base-content/70" data-testid="match-older">
            {older}.
            <button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={() => void matches.showAll()} data-testid="match-show-all"
                >Show all</button
            >
        </p>
    {/if}
{/snippet}

{#snippet groupMore(proposal: MatchProposal)}
    <span class="badge badge-ghost badge-sm" title="{proposal.group.length} postings">+{proposal.group.length - 1}</span>
{/snippet}

<div
    bind:this={root}
    class="review-layer fixed inset-0 z-[999] overflow-hidden"
    role="dialog"
    aria-modal="true"
    aria-label="Find Matches"
    data-testid="match-overlay"
    tabindex="-1"
    onkeydown={onKeydown}
    use:dismissible={{trap: true, onDismiss: requestClose}}
>
    <div class="pointer-events-none relative z-10 flex h-full flex-col items-center gap-3 px-3 pt-3 sm:gap-4 sm:px-6 sm:pt-5" out:layerOut|global={{reduced}}>
        <div
            class="pointer-events-auto relative z-20 flex w-full max-w-4xl shrink-0 items-center gap-3 rounded-box border border-base-content/10 bg-base-300/95 py-1.5 pr-1.5 pl-4 text-base-content shadow-lg backdrop-blur"
            in:barIn|global={{reduced}}
        >
            <h2 class="flex items-center gap-2 text-sm font-semibold tracking-wide"><ReviewIcon name="link" /> Find Matches</h2>
            {#if matches.scope !== null}
                <!-- Opened from an import's result (D9 "Import"): only pairs with a new half. -->
                <span class="badge badge-ghost badge-sm" data-testid="match-scope">this import</span>
            {/if}
            {#if focused}
                <span class="badge badge-ghost badge-sm" data-testid="match-focus-badge">one transaction</span>
            {/if}
            {#if started && page !== null && !focused}
                <span class="text-sm text-base-content/65 tabular-nums" data-testid="match-remaining" aria-live="polite">{queue.length} left</span>
            {/if}
            {#if busy}
                <span class="flex items-center gap-1.5 text-xs text-base-content/60" role="status" data-testid="match-saving">
                    <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-success"></span> saving…
                </span>
            {/if}
            <span class="grow"></span>
            <details
                class="dropdown dropdown-end"
                bind:open={menuOpen}
                use:dismissible={{active: menuOpen, outside: true, onDismiss: () => (menuOpen = false)}}
            >
                <summary class="btn btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10" aria-label="Match settings" title="Settings">
                    <ReviewIcon name="sliders" />
                </summary>
                <div class="dropdown-content z-30 mt-1 rounded-box border border-base-300 bg-base-200 p-3 text-base-content shadow-xl">
                    <ReviewSettings show="matches" />
                </div>
            </details>
            <button
                type="button"
                class="btn btn-square btn-ghost text-base-content/80 btn-sm hover:bg-base-content/10"
                aria-label="Close Find Matches"
                title={hint("Close", "Esc")}
                onclick={requestClose}
            >
                <ReviewIcon name="x" />
            </button>
        </div>

        {#if notice !== null}
            <div class="pointer-events-auto alert w-full max-w-4xl alert-warning shadow-lg" role="alert" data-testid="match-notice">
                <span class="grow">{notice}</span>
                <button type="button" class="btn shrink-0 btn-ghost btn-sm" onclick={() => (notice = null)}>Dismiss</button>
            </div>
        {/if}

        <div bind:this={stack} class="deck relative grid w-full max-w-4xl" data-testid="match-stack" data-motion={reduced ? "reduced" : "full"}>
            {#if focused}
                <MatchFocus bind:this={focusView} panel={PANEL} {reduced} onClose={close} />
            {:else if page === null}
                <div class="{PANEL} flex items-center justify-center gap-2 py-16 text-sm text-base-content/60" role="status">
                    {#if matches.error !== null}
                        <span class="text-error">Couldn't find matches: {matches.error}</span>
                    {:else}
                        <span class="loading loading-sm loading-spinner"></span> Finding matches…
                    {/if}
                </div>
            {:else if empty}
                <div class="{PANEL} flex flex-col items-center gap-3 px-5 py-14 text-center" data-testid="match-empty" data-match-panel tabindex="-1">
                    <span class="grid h-12 w-12 place-items-center rounded-full bg-success/15 text-success"><ReviewIcon name="check" class="h-6 w-6" /></span>
                    <p class="text-lg font-medium">Nothing to match</p>
                    <p class="max-w-md text-sm text-base-content/60">
                        {!page.clearing
                            ? "No clearing account is declared, so there is nothing to pair up."
                            : matches.scope !== null
                              ? "None of this import's transfer halves has a partner to link."
                              : "Every transfer half in the window is linked, ignored or still waiting for its other half."}
                    </p>
                    {@render olderNote()}
                    <button type="button" class="btn btn-sm" onclick={close}>Close</button>
                </div>
            {:else if !started}
                <!-- The counts screen (D9 Bulk). -->
                <div
                    class="{PANEL} flex flex-col items-center gap-4 px-5 py-12 text-center"
                    data-testid="match-intro"
                    data-match-panel
                    tabindex="-1"
                    in:cardIn|global={motion}
                >
                    <span class="grid h-12 w-12 place-items-center rounded-full bg-info/15 text-info"><ReviewIcon name="link" class="h-6 w-6" /></span>
                    {#if linkedAll !== null}
                        <p class="text-lg font-medium" data-testid="match-linked-all">Linked {linkedAll} {linkedAll === 1 ? "pair" : "pairs"}.</p>
                    {/if}
                    <p class="text-lg tabular-nums" data-testid="match-counts">
                        <span class="font-semibold text-success">{clearPairs.length} clear</span>, <span class="font-semibold">{checkCount} to check</span>
                    </p>
                    <p class="max-w-md text-sm text-base-content/60">
                        Each pair is the two halves of one transfer, landed on the same clearing account. Linking writes one <code>match:</code> tag on each half
                        and changes nothing else.
                    </p>
                    {@render olderNote()}
                    {#if failure !== null}
                        <div class="alert py-2 text-sm alert-error" role="alert" data-testid="match-failure">
                            <span>Not linked: {failure}</span>
                        </div>
                    {/if}
                    {#if confirmAll}
                        <div
                            class="flex flex-col items-center gap-2 rounded-box border border-warning/30 bg-warning/10 px-4 py-3 text-sm"
                            role="alert"
                            data-testid="match-confirm-all"
                        >
                            <span>Link all {clearPairs.length} clear matches? Each pair gets a <code>match:</code> tag on both halves.</span>
                            <span class="flex gap-2">
                                <button
                                    type="button"
                                    class="btn btn-sm btn-success"
                                    disabled={busy}
                                    onclick={() => void linkAll()}
                                    data-testid="match-confirm-all-go">Link {clearPairs.length} pairs</button
                                >
                                <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmAll = false)}>Cancel</button>
                            </span>
                        </div>
                    {:else}
                        <div class="flex flex-wrap justify-center gap-2">
                            {#if clearPairs.length > 0}
                                <button type="button" class="btn btn-success" disabled={busy} onclick={() => void linkAll()} data-testid="match-link-all">
                                    <ReviewIcon name="check" /> Link all {clearPairs.length} clear {clearPairs.length === 1 ? "match" : "matches"}
                                    <kbd class="kbd border-success-content/30 bg-success-content/10 kbd-xs text-success-content">a</kbd>
                                </button>
                            {/if}
                            {#if page.proposals.length > 0}
                                <button type="button" class="btn" onclick={start} data-testid="match-start">
                                    {clearPairs.length > 0 ? "Review one at a time" : `Review ${page.proposals.length} to check`}
                                    <ReviewIcon name="arrow-right" /> <kbd class="kbd kbd-xs">↵</kbd>
                                </button>
                            {:else}
                                <button type="button" class="btn" onclick={close}>Done</button>
                            {/if}
                        </div>
                    {/if}
                </div>
            {:else}
                {#if finished}
                    <div
                        class="{PANEL} flex flex-col items-center gap-3 px-5 py-12 text-center"
                        data-testid="match-summary"
                        data-match-panel
                        tabindex="-1"
                        in:cardIn={motion}
                        out:peekOut={{reduced}}
                    >
                        <span class="grid h-14 w-14 place-items-center rounded-full bg-success/15 text-success"
                            ><ReviewIcon name="check" class="h-7 w-7" /></span
                        >
                        <p class="text-lg font-medium">All done</p>
                        <p class="text-sm text-base-content/70 tabular-nums" data-testid="match-tally">
                            {tally.linked} linked · {tally.rejected} not a match · {tally.ignored} ignored · {tally.skipped} skipped
                        </p>
                        <button type="button" class="btn btn-sm btn-success" onclick={close}>Done <kbd class="kbd kbd-xs">↵</kbd></button>
                    </div>
                {/if}

                {#each peeks as waiting, at (pairKey(waiting.a, waiting.b))}
                    <div
                        class={[PEEK, at === 0 ? "z-[2]" : "z-[1]"]}
                        style={peekStyle(at + 1, reduced)}
                        inert
                        aria-hidden="true"
                        in:peekIn|global={{reduced}}
                        out:peekOut={{reduced}}
                    >
                        <p class="text-2xl font-semibold">{waiting.from.label ?? "?"} → {waiting.to.label ?? "?"}</p>
                    </div>
                {/each}

                {#key cardKey}
                    {#if proposal !== null && choice !== null && header !== null}
                        <article
                            class={[CARD, "border-base-content/10"]}
                            data-card-key={cardKey}
                            data-testid="match-card"
                            tabindex="-1"
                            aria-label="{header.from} to {header.to}"
                            in:cardIn|global={motion}
                            out:cardOut|global={motion}
                        >
                            <div class="flex min-h-0 flex-col overflow-y-auto overscroll-contain">
                                <header class="flex items-start justify-between gap-6 px-5 pt-5 sm:px-8 sm:pt-7">
                                    <div class="min-w-0">
                                        <h2
                                            class="flex flex-wrap items-center gap-2 text-2xl leading-tight font-semibold tracking-tight"
                                            data-testid="match-header"
                                        >
                                            <ReviewIcon name="link" class="h-6 w-6 text-info" />
                                            <span title={header.fromAccount ?? undefined}>{header.from}</span>
                                            {#if combination && proposal.from.index !== proposal.a.txn.index}{@render groupMore(proposal)}{/if}
                                            <span class="text-base-content/40" aria-hidden="true">→</span>
                                            <span title={header.toAccount ?? undefined}>{header.to}</span>
                                            {#if combination && proposal.to.index !== proposal.a.txn.index}{@render groupMore(proposal)}{/if}
                                        </h2>
                                        <p class="mt-1.5 flex items-center gap-2 text-sm text-base-content/60">
                                            <span
                                                class={["badge badge-sm", proposal.clear && choiceAt === 0 ? "badge-success" : "badge-warning"]}
                                                data-testid="match-confidence">{proposal.clear && choiceAt === 0 ? "clear" : "check"}</span
                                            >
                                            <span data-testid="match-gap">{gapLabel(choice.gapDays)}</span>
                                            {#if combination}
                                                <span data-testid="match-combination">· one posting against {proposal.group.length}</span>
                                            {/if}
                                        </p>
                                    </div>
                                    <div
                                        class="shrink-0 font-mono text-3xl leading-none font-light tracking-tight text-info tabular-nums sm:text-4xl"
                                        data-testid="match-amount"
                                    >
                                        {header.amount ?? "—"}
                                    </div>
                                </header>

                                {#if choices.length > 1}
                                    <fieldset class="mx-5 mt-4 rounded-box border border-warning/30 bg-warning/5 px-4 py-3 sm:mx-8" data-testid="match-choices">
                                        <legend class="px-1 text-xs font-medium text-base-content/70">More than one half fits. Pair with:</legend>
                                        {#each choices as option, at (pairKey(option.a, option.b))}
                                            {@const other = otherHalf(option, proposal)}
                                            <label class="flex cursor-pointer items-center gap-2 py-0.5 text-sm">
                                                <input
                                                    type="radio"
                                                    class="radio radio-xs"
                                                    name="match-choice"
                                                    checked={choiceAt === at}
                                                    onchange={() => (choiceAt = at)}
                                                />
                                                <span class="tabular-nums">{longDate(other.txn.date)}</span>
                                                <span class="truncate">{other.txn.description}</span>
                                                <span class="shrink-0 text-xs text-base-content/50"
                                                    >{gapLabel(option.gapDays)}{option.taken ? " · in another pair" : ""}</span
                                                >
                                            </label>
                                        {/each}
                                    </fieldset>
                                {/if}

                                {#if failure !== null}
                                    <div class="mx-5 mt-4 alert py-2 text-sm alert-error sm:mx-8" role="alert" data-testid="match-failure">
                                        <span class="grow break-words">Not saved: {failure}</span>
                                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => (failure = null)}>Dismiss</button>
                                    </div>
                                {/if}

                                <div class="mt-4 grid grid-cols-1 divide-base-content/10 border-t border-base-content/10 sm:grid-cols-2 sm:divide-x">
                                    {#key `${choice.a.txn.index}:${choice.b.txn.index}`}
                                        <MatchHalfView
                                            half={choice.a}
                                            side="left"
                                            bind:description={() => describe(choice.a), (value) => (drafts[choice.a.txn.index] = value)}
                                            payees={reviewLaunch.payees}
                                            {today}
                                            disabled={busy}
                                        />
                                        {#if combination}
                                            <section class="flex min-w-0 flex-col gap-2 p-5 sm:p-6" data-testid="match-group">
                                                <p class="text-sm text-base-content/70">
                                                    {proposal.group.length} postings that add up to it, via {halfAccount(proposal.group[0])}
                                                </p>
                                                <ul class="flex flex-col divide-y divide-base-content/10">
                                                    {#each proposal.group as member (pairKey(member, proposal.a))}
                                                        <li class="py-1.5"><MatchLine half={member} /></li>
                                                    {/each}
                                                </ul>
                                            </section>
                                        {:else}
                                            <MatchHalfView
                                                half={choice.b}
                                                side="right"
                                                bind:description={() => describe(choice.b), (value) => (drafts[choice.b.txn.index] = value)}
                                                payees={reviewLaunch.payees}
                                                {today}
                                                disabled={busy}
                                            />
                                        {/if}
                                    {/key}
                                </div>
                            </div>

                            {#if confirmClose}
                                <div
                                    class="flex min-h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-warning/30 bg-warning/10 px-5 py-3 text-sm sm:px-8"
                                    role="alert"
                                    data-testid="match-confirm-close"
                                >
                                    <span>The descriptions on this card have unsaved changes.</span>
                                    <span class="flex gap-2">
                                        <button type="button" class="btn btn-ghost btn-sm" onclick={() => ((confirmClose = false), void focusCard())}
                                            >Keep editing</button
                                        >
                                        <button type="button" class="btn btn-sm btn-warning" onclick={close}>Discard and close</button>
                                    </span>
                                </div>
                            {:else if confirmIgnore !== null}
                                <div
                                    class="flex min-h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-warning/30 bg-warning/10 px-5 py-3 text-sm sm:px-8"
                                    role="alert"
                                    data-testid="match-confirm-ignore"
                                >
                                    <span class="min-w-0">
                                        Never propose <strong class="font-medium">{sides[confirmIgnore]}</strong> again? It writes <code>match:ignore</code> on its
                                        clearing posting.
                                    </span>
                                    <span class="flex shrink-0 gap-2">
                                        <button
                                            type="button"
                                            class="btn btn-sm btn-warning"
                                            disabled={busy}
                                            onclick={() => confirmIgnore !== null && void ignore(confirmIgnore)}
                                            data-testid="match-confirm-ignore-go">Ignore this side</button
                                        >
                                        <button type="button" class="btn btn-ghost btn-sm" onclick={() => ((confirmIgnore = null), void focusCard())}
                                            >Keep</button
                                        >
                                    </span>
                                </div>
                            {:else}
                                <MatchFooter
                                    {dirty}
                                    {reduced}
                                    {busy}
                                    canReject={!combination}
                                    {sides}
                                    onLink={() => void link()}
                                    onReject={() => void reject()}
                                    onSkip={skip}
                                    onRevert={revert}
                                    onIgnore={(side) => void ignore(side)}
                                />
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
        aria-label="Close Find Matches"
        onclick={requestClose}
        in:backdrop|global={{reduced}}
        out:backdrop|global={{reduced, closing: true}}
    ></button>
</div>

<style>
    /* As the review overlay's deck: the outgoing card leaves the grid's flow and flies over the incoming one. */
    .review-layer:not([data-closing]) :global(.deck-card[inert]) {
        position: absolute;
        inset-inline: 0;
        top: 0;
        z-index: 6;
    }
</style>
