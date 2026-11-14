<script lang="ts">
    // Find match for ONE transaction (plan 35 Amendment A2), in the Find
    // Matches overlay's frame: what `GET /api/matches/for/{index}` offers each
    // of its unlinked clearing postings.
    //
    //   ⇄ Find a match                                       $2,247.50
    //     Feb 19, 2019 · BILL.COM PAYABLES · via transfers:billpay
    //   ( ) Exact amount, any date        Feb 20 · ACME INV 12 · 1 day apart
    //   ( ) Combinations                  Feb 18 · … + Feb 21 · …
    //   [ ] Nearby, ± 10 days [10]        Feb 18 · ACME INV 12  $2,000.00
    //   [ ]                               Feb 21 · BOLT INV 7     $247.50
    //   $2,247.50 of $2,247.50 ✓
    //   ✓ Link                            ✕ Not a match   Ignore this side
    //
    // The radios and the checkboxes edit ONE selection (the postings to link
    // with this one): an exact partner or a combination is a preset of it,
    // and ticking nearby postings builds one by hand. Link is enabled only
    // when the selection sums to exactly zero with this posting (decimal
    // arithmetic, `selectionTotal`). One partner links as a pair; more as a
    // group (A3). Not a match is for a single exact partner; Ignore this side
    // is always offered. The transaction's links are listed with Unlink.
    //
    // Writes go through the `matches` store, which re-reads this view. A link
    // or an ignore that leaves the transaction nothing more to match closes
    // the overlay; otherwise the view stays on what is left.
    import {untrack} from "svelte";
    import {formatAmount, neg} from "#lib/domain/money.js";
    import {keymap, registerKeys} from "#lib/keys/keymap.svelte.js";
    import {isTypingTarget, type TargetLike} from "#lib/keys/target.js";
    import {PRIORITY} from "#lib/keys/types.js";
    import {longDate} from "#lib/review/card.js";
    import {cardIn, type MotionParams} from "#lib/review/ui/motion.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {localToday} from "#lib/stores/filters.svelte.js";
    import {matches, type MatchOutcome} from "#lib/stores/matches.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {clearingAmount, combinationMembers, focusedHalf, focusLinkBody, halfKey, halfView, partnerFlags, selectionTotal, wireHalf} from "../model";
    import type {MatchHalf, MatchLink} from "../types";
    import MatchLine from "./MatchLine.svelte";

    let {panel, reduced, onClose}: {panel: string; reduced: boolean; onClose: () => void} = $props();

    const today = localToday();
    const uid = $props.id();
    const motion: MotionParams = untrack(() => ({exit: "open", reduced}));

    let root = $state<HTMLElement | null>(null);
    /** Which of the transaction's unlinked clearing postings is being matched. */
    let postingAt = $state(0);
    /** The selection: the halves (by `halfKey`) to link with the focused posting. */
    let picked = $state.raw<string[]>([]);
    type Armed = {kind: "ignore"} | {kind: "unlink"; link: MatchLink};
    let armed = $state.raw<Armed | null>(null);
    let failure = $state<string | null>(null);
    let notice = $state<string | null>(null);
    let seededFor: string | null = null;

    const focus = $derived(matches.focus);
    const data = $derived(focus?.data ?? null);
    const entry = $derived(data?.postings[postingAt] ?? data?.postings[0] ?? null);
    const focused = $derived(data === null || entry === null ? null : focusedHalf(data.transaction, entry.posting));
    const busy = $derived(matches.busy);
    const own = $derived(focused === null ? null : clearingAmount(focused));
    /** What a partner must carry: this posting's amount negated. */
    const need = $derived(own === null ? null : {...own, qty: neg(own.qty)});
    /** The amount moved, unsigned: the header's figure. */
    const moved = $derived(own === null || need === null ? null : own.qty.m < 0n ? need : own);
    const clearingAccount = $derived(focused === null ? null : (halfView(focused).clearing?.account ?? null));

    /** One radio choice: an exact partner, or a combination (its members other than this posting). */
    interface Option {
        id: string;
        keys: string[];
        kind: "exact" | "combination";
    }
    const options = $derived.by((): Option[] => {
        if (entry === null || focused === null) return [];
        const self = halfKey(focused);
        return [
            ...entry.exact.map((partner) => ({id: `e:${halfKey(partner)}`, keys: [halfKey(partner)], kind: "exact" as const})),
            ...entry.combinations.map((proposal) => {
                const keys = combinationMembers(proposal)
                    .map(halfKey)
                    .filter((key) => key !== self);
                return {id: `c:${keys.join("+")}`, keys, kind: "combination" as const};
            }),
        ];
    });
    /** Every half the view offers, by key. */
    const byKey = $derived.by(() => {
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- rebuilt wholesale inside $derived.by, never mutated afterwards
        const halves = new Map<string, MatchHalf>();
        if (entry === null) return halves;
        for (const half of [...entry.exact, ...entry.nearby, ...entry.combinations.flatMap(combinationMembers)]) halves.set(halfKey(half), half);
        return halves;
    });
    const pickedHalves = $derived(picked.flatMap((key) => byKey.get(key) ?? []));
    const total = $derived(focused === null ? null : selectionTotal(focused, pickedHalves));
    const exactKeys = $derived(new Set(entry?.exact.map(halfKey) ?? []));
    const canReject = $derived(picked.length === 1 && exactKeys.has(picked[0] ?? ""));
    const canLink = $derived(total?.balanced === true && !busy && armed === null);
    const nothing = $derived(entry !== null && entry.exact.length === 0 && entry.combinations.length === 0 && entry.nearby.length === 0);

    const sameSet = (a: readonly string[], b: readonly string[]): boolean => a.length === b.length && a.every((key) => b.includes(key));
    const chosenAt = $derived(options.findIndex((option) => sameSet(option.keys, picked)));

    // Each read of a posting (a fresh answer, another posting): the nearest
    // exact partner picked, else the first combination, else nothing.
    $effect(() => {
        const key = data === null || entry === null ? null : `${data.generation}|${focus?.index}|${entry.posting}`;
        untrack(() => {
            if (key === seededFor) return;
            seededFor = key;
            picked = options[0]?.keys ?? [];
            armed = null;
        });
    });

    function choose(option: Option): void {
        picked = option.keys;
    }

    function toggle(key: string, on: boolean): void {
        picked = on ? [...picked.filter((other) => other !== key), key] : picked.filter((other) => other !== key);
    }

    /** `c`: the next radio choice. */
    function cycle(): void {
        if (options.length === 0) return;
        const next = options[(chosenAt + 1) % options.length];
        if (next !== undefined) choose(next);
    }

    function pickPosting(at: number): void {
        postingAt = at;
        failure = null;
    }

    /**
     * A write's outcome. A conflict re-read everything (the store does that
     * either way) and says so; another failure stays on the view. Linking or
     * ignoring the transaction's last open posting closes the overlay.
     */
    async function settle(outcome: Promise<MatchOutcome>, closeWhenDone: boolean): Promise<void> {
        failure = null;
        armed = null;
        const result = await outcome;
        if (!result.ok) {
            if (result.failure.kind === "conflict")
                notice = "The journal changed since this was read, so nothing was written. It has been read again: check and try again.";
            else failure = result.failure.message;
            return;
        }
        notice = null;
        const left = matches.focus?.data?.postings.length ?? 0;
        if (closeWhenDone && left === 0) return onClose();
        if (postingAt >= left) postingAt = 0;
    }

    function link(): void {
        if (!canLink || focused === null) return;
        void settle(matches.link(focusLinkBody(focused, pickedHalves)), true);
    }

    function reject(): void {
        const partner = pickedHalves[0];
        if (!canReject || busy || focused === null || partner === undefined) return;
        void settle(matches.reject({a: wireHalf(focused), b: wireHalf(partner)}), false);
    }

    /** Ignore this side: arm, then confirm (it is permanent, D1). */
    function ignore(): void {
        if (focused === null || busy) return;
        if (armed?.kind !== "ignore") {
            armed = {kind: "ignore"};
            return;
        }
        void settle(matches.ignore(wireHalf(focused)), true);
    }

    /** Unlink: arm, then confirm. Every member of the link is named. */
    function unlink(target: MatchLink): void {
        if (busy) return;
        if (armed?.kind !== "unlink" || armed.link.matchId !== target.matchId) {
            armed = {kind: "unlink", link: target};
            return;
        }
        void settle(matches.unlinkLink(target), false);
    }

    function setWindow(raw: string): void {
        const days = Number(raw);
        if (raw.trim() === "" || !Number.isFinite(days)) return;
        void matches.setFocusWindow(days);
    }

    function activateFocused(): boolean {
        const target = document.activeElement;
        if (target instanceof HTMLButtonElement || (target instanceof HTMLInputElement && (target.type === "radio" || target.type === "checkbox"))) {
            if (root?.contains(target) === true) {
                target.click();
                return true;
            }
        }
        return false;
    }

    /** Escape (and the overlay's dismiss): cancel an armed confirm, else close. */
    export function escape(): void {
        if (armed !== null) {
            armed = null;
            root?.focus();
            return;
        }
        onClose();
    }

    /**
     * The overlay's element-level keydown, before the window keymap (which
     * ignores fields): Escape leaves a field, ⌘↵ links from one.
     */
    export function onKeydown(event: KeyboardEvent): void {
        if (event.defaultPrevented || event.isComposing) return;
        if (event.key === "Escape" && isTypingTarget(event.target as TargetLike | null)) {
            event.preventDefault();
            root?.focus();
            return;
        }
        if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.altKey) {
            event.preventDefault();
            link();
        }
    }

    const live = (): boolean => !keymap.helpOpen && !busy;

    // One above the overlay's own layer: mounted with the overlay, this
    // child's layer registers FIRST, and the overlay's modal layer would
    // otherwise blind it.
    registerKeys({
        id: "match-focus",
        priority: PRIORITY.overlay + 1,
        modal: true,
        bindings: [
            {keys: "mod+Enter", label: "Link the selection", group: "Matches", enabled: live, run: link},
            {
                keys: "Enter",
                label: "Link the selection, or the focused button",
                group: "Matches",
                enabled: live,
                run: () => activateFocused() || (armed?.kind === "ignore" ? ignore() : armed?.kind === "unlink" ? unlink(armed.link) : link()),
            },
            {keys: "y", label: "Link the selection", group: "Matches", enabled: () => live() && canLink, run: link},
            {keys: "x", label: "Not a match (one exact partner)", group: "Matches", enabled: () => live() && canReject, run: reject},
            {keys: "c", label: "Choose the next exact partner or combination", group: "Matches", enabled: () => live() && options.length > 1, run: cycle},
            {keys: "i", label: "Ignore this side (asks first)", group: "Matches", enabled: () => live() && focused !== null, run: ignore},
            {keys: "Escape", label: "Leave the field, cancel, or close", group: "Matches", run: escape},
            {keys: "?", label: "Show keyboard shortcuts", group: "Matches", run: () => keymap.toggleHelp()},
        ],
    });
</script>

{#snippet windowControl()}
    <label class="flex items-center gap-1.5 text-xs text-base-content/60" for="{uid}-window">
        within ±
        <input
            id="{uid}-window"
            type="number"
            min="1"
            max="60"
            class="input w-16 text-right tabular-nums input-xs"
            value={matches.focusWindow}
            onchange={(event) => setWindow(event.currentTarget.value)}
            aria-label="Days either side to look for nearby postings"
            data-testid="match-focus-window"
        />
        days
    </label>
{/snippet}

{#snippet linksList(links: readonly MatchLink[])}
    <section class="flex flex-col gap-3" data-testid="match-focus-links">
        <h3 class="text-xs font-medium tracking-wide text-base-content/55 uppercase">Linked</h3>
        {#each links as link (link.matchId)}
            <div class="rounded-box border border-base-content/10 px-4 py-3" data-testid="match-focus-link" data-match-id={link.matchId}>
                <div class="flex items-center gap-2 text-sm">
                    <ReviewIcon name="link" class="h-4 w-4 text-info" />
                    <code>match:{link.matchId}</code>
                    <span class="text-base-content/55">· {link.members.length} postings</span>
                    {#if link.broken}<span class="badge badge-sm badge-warning">broken</span>{/if}
                    <span class="grow"></span>
                    {#if armed?.kind === "unlink" && armed.link.matchId === link.matchId}
                        <span class="text-xs text-base-content/70">Remove <code>match:{link.matchId}</code> from all {link.members.length}?</span>
                        <button type="button" class="btn btn-warning btn-xs" disabled={busy} onclick={() => unlink(link)} data-testid="match-focus-unlink-go"
                            >Unlink</button
                        >
                        <button type="button" class="btn btn-ghost btn-xs" onclick={() => (armed = null)}>Keep</button>
                    {:else}
                        <button type="button" class="btn gap-1 btn-ghost btn-xs" disabled={busy} onclick={() => unlink(link)} data-testid="match-focus-unlink">
                            <ReviewIcon name="unlink" class="h-3.5 w-3.5" /> Unlink
                        </button>
                    {/if}
                </div>
                <ul class="mt-2 flex flex-col divide-y divide-base-content/10">
                    {#each link.members as member (halfKey(member))}
                        <li class="py-1"><MatchLine half={member} /></li>
                    {/each}
                </ul>
            </div>
        {/each}
    </section>
{/snippet}

<article
    bind:this={root}
    class="{panel} flex max-h-[calc(100dvh-7.5rem-var(--ll-top-inset))] flex-col"
    data-testid="match-focus"
    data-match-panel
    tabindex="-1"
    aria-label="Find a match for one transaction"
    in:cardIn|global={motion}
>
    {#if focus === null || (data === null && focus.error === null)}
        <div class="flex items-center justify-center gap-2 py-16 text-sm text-base-content/60" role="status">
            <span class="loading loading-sm loading-spinner"></span> Finding matches for this transaction…
        </div>
    {:else if data === null}
        <div class="flex flex-col items-center gap-3 py-14 text-sm" role="alert" data-testid="match-focus-error">
            <span class="text-error">Couldn't find matches: {focus.error}</span>
            <span class="flex gap-2">
                <button type="button" class="btn btn-sm" onclick={() => void matches.reloadFocus()}>Try again</button>
                <button type="button" class="btn btn-ghost btn-sm" onclick={onClose}>Close</button>
            </span>
        </div>
    {:else}
        <div class="flex min-h-0 flex-col gap-5 overflow-y-auto overscroll-contain p-5 sm:p-8">
            <header class="flex items-start justify-between gap-6">
                <div class="min-w-0">
                    <h2 class="flex items-center gap-2 text-2xl leading-tight font-semibold tracking-tight">
                        <ReviewIcon name="link" class="h-6 w-6 text-info" /> Find a match
                    </h2>
                    <p class="mt-1.5 truncate text-sm text-base-content/65" data-testid="match-focus-txn">
                        {longDate(data.transaction.txn.date)} · {data.transaction.txn.description || "(no description)"}
                        {#if clearingAccount !== null}<span class="text-base-content/50"> · via {clearingAccount}</span>{/if}
                    </p>
                </div>
                {#if moved !== null}
                    <div class="shrink-0 font-mono text-3xl leading-none font-light tracking-tight text-info tabular-nums" data-testid="match-focus-amount">
                        {formatAmount(moved)}
                    </div>
                {/if}
            </header>

            {#if notice !== null}
                <div class="alert py-2 text-sm alert-warning" role="alert" data-testid="match-notice">
                    <span class="grow">{notice}</span>
                    <button type="button" class="btn btn-ghost btn-xs" onclick={() => (notice = null)}>Dismiss</button>
                </div>
            {/if}
            {#if failure !== null}
                <div class="alert py-2 text-sm alert-error" role="alert" data-testid="match-failure">
                    <span class="grow break-words">Not saved: {failure}</span>
                    <button type="button" class="btn btn-ghost btn-xs" onclick={() => (failure = null)}>Dismiss</button>
                </div>
            {/if}

            {#if data.postings.length > 1}
                <div class="flex flex-wrap gap-1" role="group" aria-label="Which posting to match">
                    {#each data.postings as one, at (one.posting)}
                        {@const half = focusedHalf(data.transaction, one.posting)}
                        {@const amount = clearingAmount(half)}
                        <button
                            type="button"
                            class={["btn btn-xs", at === postingAt ? "btn-active btn-primary" : "btn-ghost"]}
                            aria-pressed={at === postingAt}
                            onclick={() => pickPosting(at)}
                            data-testid="match-focus-posting"
                        >
                            {halfView(half).clearing?.account ?? "?"}
                            {amount === null ? "" : formatAmount(amount)}
                        </button>
                    {/each}
                </div>
            {/if}

            {#if entry === null}
                <!-- Nothing open to match: linked, ignored, or not a transfer half at all. -->
                <p class="text-sm text-base-content/70" data-testid="match-focus-none">
                    {!data.clearing
                        ? "No clearing account is declared, so there is nothing to match."
                        : data.links.length > 0
                          ? "Every transfer half of this transaction is linked."
                          : "This transaction has no unlinked transfer half to match."}
                </p>
            {:else if nothing}
                <div class="flex flex-col gap-3 rounded-box border border-base-content/10 px-4 py-4" data-testid="match-focus-empty">
                    <p class="text-sm">
                        No unlinked {need === null ? "opposite amount" : formatAmount(need)} on
                        <code>{clearingAccount ?? "this clearing account"}</code> at any date, and nothing opposite within ±{matches.focusWindow} days.
                    </p>
                    <p class="text-xs text-base-content/60">
                        The other half may not be imported yet. If it never will be, Ignore this side takes it off the in-transit list.
                    </p>
                    {@render windowControl()}
                </div>
            {:else}
                <fieldset class="flex flex-col gap-4" disabled={busy}>
                    <legend class="sr-only">What to link this posting with</legend>
                    {#if entry.exact.length > 0}
                        <section class="flex flex-col gap-1" data-testid="match-focus-exact">
                            <h3 class="text-xs font-medium tracking-wide text-base-content/55 uppercase">Exact amount, any date</h3>
                            {#each entry.exact as partner (halfKey(partner))}
                                {@const option = options.find((candidate) => candidate.id === `e:${halfKey(partner)}`)}
                                <label class="flex cursor-pointer items-center gap-3 rounded px-1 py-1 hover:bg-base-200/60">
                                    <input
                                        type="radio"
                                        class="radio shrink-0 radio-xs"
                                        name="{uid}-option"
                                        checked={option !== undefined && sameSet(option.keys, picked)}
                                        onchange={() => option !== undefined && choose(option)}
                                        data-testid="match-focus-exact-option"
                                    />
                                    <MatchLine
                                        half={partner}
                                        gapDays={partner.gapDays}
                                        flags={partnerFlags(partner, today, settings.matchWindow, settings.matchLookback)}
                                    />
                                </label>
                            {/each}
                        </section>
                    {/if}
                    {#if entry.combinations.length > 0}
                        <section class="flex flex-col gap-1" data-testid="match-focus-combinations">
                            <h3 class="text-xs font-medium tracking-wide text-base-content/55 uppercase">Combinations that add up</h3>
                            {#each entry.combinations as proposal (proposal.group.map(halfKey).join("+"))}
                                {@const keys = combinationMembers(proposal)
                                    .map(halfKey)
                                    .filter((key) => focused === null || key !== halfKey(focused))}
                                <label class="flex cursor-pointer items-start gap-3 rounded px-1 py-1 hover:bg-base-200/60">
                                    <input
                                        type="radio"
                                        class="radio mt-1 shrink-0 radio-xs"
                                        name="{uid}-option"
                                        checked={sameSet(keys, picked)}
                                        onchange={() => (picked = keys)}
                                        data-testid="match-focus-combination-option"
                                    />
                                    <span class="flex min-w-0 grow flex-col">
                                        {#each keys as key (key)}
                                            {@const member = byKey.get(key)}
                                            {#if member !== undefined}<MatchLine half={member} />{/if}
                                        {/each}
                                    </span>
                                </label>
                            {/each}
                        </section>
                    {/if}
                    <section class="flex flex-col gap-1" data-testid="match-focus-nearby">
                        <div class="flex flex-wrap items-center justify-between gap-2">
                            <h3 class="text-xs font-medium tracking-wide text-base-content/55 uppercase">Pick several</h3>
                            {@render windowControl()}
                        </div>
                        {#if entry.nearby.length === 0}
                            <p class="px-1 text-xs text-base-content/55">Nothing opposite on this account within ±{matches.focusWindow} days.</p>
                        {/if}
                        {#each entry.nearby as partner (halfKey(partner))}
                            <label class="flex cursor-pointer items-center gap-3 rounded px-1 py-1 hover:bg-base-200/60">
                                <input
                                    type="checkbox"
                                    class="checkbox shrink-0 checkbox-xs"
                                    checked={picked.includes(halfKey(partner))}
                                    onchange={(event) => toggle(halfKey(partner), event.currentTarget.checked)}
                                    data-testid="match-focus-nearby-option"
                                />
                                <MatchLine half={partner} gapDays={partner.gapDays} />
                            </label>
                        {/each}
                    </section>
                </fieldset>
                {#if total !== null && picked.length > 0}
                    <p
                        class={["flex items-center gap-2 text-sm tabular-nums", total.balanced ? "text-success" : "text-base-content/70"]}
                        data-testid="match-focus-total"
                        data-balanced={total.balanced}
                        aria-live="polite"
                    >
                        {total.label}
                        {#if total.balanced}<ReviewIcon name="check" class="h-4 w-4" />{/if}
                        {#if total.problem !== null}<span class="text-warning">{total.problem}</span>{/if}
                    </p>
                {/if}
            {/if}

            {#if data.links.length > 0}
                {@render linksList(data.links)}
            {/if}
        </div>

        {#if armed?.kind === "ignore"}
            <div
                class="flex min-h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-warning/30 bg-warning/10 px-5 py-3 text-sm sm:px-8"
                role="alert"
                data-testid="match-confirm-ignore"
            >
                <span class="min-w-0">Never propose this posting again? It writes <code>match:ignore</code> on it.</span>
                <span class="flex shrink-0 gap-2">
                    <button type="button" class="btn btn-sm btn-warning" disabled={busy} onclick={ignore} data-testid="match-confirm-ignore-go"
                        >Ignore this side</button
                    >
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (armed = null)}>Keep</button>
                </span>
            </div>
        {:else}
            <footer class="flex h-[4.5rem] shrink-0 items-center justify-between gap-3 border-t border-base-content/10 px-5 sm:px-8" data-testid="match-footer">
                <span class="flex items-center gap-2">
                    {#if entry !== null && !nothing}
                        <button type="button" class="btn gap-2 btn-success" onclick={link} disabled={!canLink} data-testid="match-link">
                            <ReviewIcon name="check" />
                            {pickedHalves.length > 1 ? `Link ${pickedHalves.length + 1} postings` : "Link"}
                            <kbd class="kbd border-success-content/30 bg-success-content/10 kbd-xs text-success-content">y</kbd>
                        </button>
                    {/if}
                </span>
                <span class="flex items-center gap-1">
                    {#if canReject}
                        <button
                            type="button"
                            class="btn gap-2 btn-ghost"
                            onclick={reject}
                            disabled={busy}
                            title="Remember these are not a pair"
                            data-testid="match-reject"
                        >
                            <ReviewIcon name="x" /> Not a match <kbd class="kbd kbd-xs">x</kbd>
                        </button>
                    {/if}
                    {#if entry !== null}
                        <button type="button" class="btn gap-2 btn-ghost" onclick={ignore} disabled={busy} data-testid="match-focus-ignore">
                            <ReviewIcon name="ban" /> Ignore this side <kbd class="kbd kbd-xs">i</kbd>
                        </button>
                    {/if}
                    <button type="button" class="btn btn-ghost" onclick={onClose} data-testid="match-focus-close">Close</button>
                </span>
            </footer>
        {/if}
    {/if}
</article>
