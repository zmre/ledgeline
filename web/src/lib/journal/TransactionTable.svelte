<script lang="ts">
    // Virtualized transaction list (WP-03): simple fixed-pitch windowing over a
    // scroll container (see computeWindow in rowModel.ts) — the rendered-row
    // count is bounded by viewport/pitch + overscan, independent of the row
    // count, which keeps 200k rows smooth. Desktop renders a daisyUI table with a
    // sticky header + spacer rows; narrow widths (<640px) render card-per-txn.
    //
    // The rows themselves are WINDOWED too (plan 26 B5): the journal view store
    // fetches 200-row blocks from the engine as the window reaches them, and a
    // row whose block has not arrived renders as a skeleton of the same height,
    // so the scroll geometry never moves under the user.
    //
    // Transfers (plan 35 D11): the engine serves a linked pair as ONE row. It
    // expands in place (▸, `l`) into its two halves, two more slots of the same
    // pitch, so the windowing stays fixed-pitch: `slotAt` maps a display slot
    // to the engine row (or half) it shows, and the cursor walks slots.
    //
    // The calendar (plan 40) takes the list's place when the filter says so:
    // the toolbar stays, the table's own keys stand down, and the toggle next
    // to the column menu switches between the two. Below the `md` breakpoint
    // there is no calendar (plan 42): no toggle, no `v`, and a `cal=1` filter
    // (a link, a restored URL) shows its month as the list.
    import {untrack} from "svelte";
    import {hint} from "#lib/keys/hint.svelte.js";
    import {statusPatch} from "#lib/api/editMapping.js";
    import type {Transaction} from "#lib/domain/types.js";
    import type {JournalRow} from "#lib/journalView/types.js";
    import {dismissible} from "#lib/keys/dismissible.js";
    import {registerKeys} from "#lib/keys/keymap.svelte.js";
    import {PRIORITY, type Binding} from "#lib/keys/types.js";
    import {registerNativeActions, type NativeAction} from "#lib/native/bridge.svelte.js";
    import {rowMarker, type RowMarker} from "#lib/review/card.js";
    import {REVIEW_BUTTON} from "#lib/review/ui/buttons.js";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import ReviewSettings from "#lib/review/ui/ReviewSettings.svelte";
    import {editing} from "#lib/stores/editing.svelte.js";
    import {filters, localToday} from "#lib/stores/filters.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {journalView} from "#lib/stores/journalView.svelte.js";
    import {findableClearing, olderLabel} from "#lib/matching/model.js";
    import {matches} from "#lib/stores/matches.svelte.js";
    import {problems} from "#lib/stores/problems.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {rootPx} from "#lib/theme/scale.js";
    import {lazyListCursor} from "#lib/ui/listCursor.svelte.js";
    import CalendarView from "./CalendarView.svelte";
    import ColumnMenu from "./ColumnMenu.svelte";
    import {calendarFits, monthOf} from "./calendarModel";
    import TransactionRow from "./TransactionRow.svelte";
    import {measureAccountColumn} from "./accountColumn.svelte";
    import {txnModal} from "./edit/modalState.svelte";
    import {rowActions} from "./rowAction.svelte";
    import {centerOffset, computeWindow, displayTotal, expandedWithin, halfPageRows, listGeometry, nearestOffset, nextStatus, slotAt, slotOf} from "./rowModel";

    /** Where the Find Matches docs live (plan 35 Phase 4), for the disabled button's "how". */
    const MATCHING_DOCS = "https://zmre.github.io/ledgeline/journal/transfer-matching/";

    const OVERSCAN = 12;

    /**
     * The section's measured width in px; null until measured (the first
     * frame, and jsdom, which never measures), when the calendar is allowed.
     * Measured on the section rather than the list's scroller because the
     * calendar replaces the scroller, and the answer must not depend on which
     * of the two is showing.
     */
    let sectionWidth = $state<number | null>(null);
    /** Whether a month grid fits (plan 42): at or above `md`, in rem at the live scale. */
    const calendarAllowed = $derived(sectionWidth === null || calendarFits(sectionWidth, rootPx()));
    /** The month calendar is showing instead of the list (plan 40). */
    const calendar = $derived(filters.value.calendar === true && calendarAllowed);

    let scroller = $state<HTMLDivElement | null>(null);
    let scrollTop = $state(0);
    let viewportHeight = $state(600);
    let containerWidth = $state(1024);
    let headHeight = $state(0);

    // Rem row geometry at the live root size (`listGeometry`), so a zoom
    // re-derives the window, and EFFECT 2 re-anchors the cursor.
    const geometry = $derived(listGeometry(containerWidth, rootPx()));
    const mode = $derived(geometry.mode);
    const pitch = $derived(geometry.pitch);
    const rowHeight = $derived(geometry.rowHeight);
    const topInset = $derived(geometry.topInset);
    // The sticky <thead> paints over the top of the viewport in table mode; card
    // mode has no header at all.
    const headroom = $derived(mode === "card" ? 0 : headHeight);
    /** Rows the filter keeps; 0 until the engine has said. */
    const total = $derived(journalView.total ?? 0);

    /**
     * The expanded transfer rows, by engine position and the link they showed.
     * One that no longer shows that link there (an unlink, a write that moved
     * rows) quietly closes; one whose block is still loading stays open, so
     * the geometry does not jump while it loads.
     */
    let expanded = $state.raw<{position: number; matchId: string}[]>([]);
    const openPositions = $derived(
        expandedWithin(
            expanded
                .filter(({position, matchId}) => {
                    const row = journalView.rowAt(position);
                    return row === null || row.transfer?.matchId === matchId;
                })
                .map(({position}) => position),
            total
        )
    );
    /** Display slots: every row, plus two for each expanded transfer. */
    const shown = $derived(displayTotal(total, openPositions));
    const win = $derived(computeWindow(scrollTop, viewportHeight, pitch, shown, OVERSCAN));

    interface Slot {
        /** The display slot. */
        at: number;
        /** The engine row it belongs to. */
        position: number;
        /** Which half of that (transfer) row it shows, or null for the row itself. */
        half: number | null;
        /** The engine row, or null while its block loads (a skeleton). */
        row: JournalRow | null;
        /** What the slot renders: the row, or the half. */
        shows: JournalRow | null;
    }
    function slotFor(at: number): Slot {
        const {position, half} = slotAt(at, openPositions);
        const row = journalView.rowAt(position);
        const shows = half === null ? row : (row?.transfer?.halves[half] ?? null);
        return {at, position, half, row, shows};
    }
    /** One slot per rendered position. */
    const visible = $derived.by((): Slot[] => {
        const slots: Slot[] = [];
        for (let at = win.start; at < win.end; at += 1) slots.push(slotFor(at));
        return slots;
    });
    /** Svelte `{#each}` key: the transaction once loaded (a half by its row's), else its slot (a skeleton never collides with a tindex). */
    const slotKey = (slot: Slot): string =>
        slot.row === null || slot.shows === null ? `p${slot.at}` : slot.half === null ? `t${slot.row.txn.index}` : `h${slot.row.txn.index}-${slot.half}`;

    // Ask for the rows the window covers. Keyed on the window and on the view's
    // QUERY (a generation change re-reads the same window), deliberately NOT on
    // the rows themselves — an effect that refetched whenever a block landed
    // would chase its own tail.
    $effect(() => {
        void journalView.query;
        const {start, end} = win;
        untrack(() => journalView.ensure(start, end));
    });
    const columns = $derived(settings.columns);

    // Review (plan 32 D18–D20): the badges' counts come with the view's summary
    // (they ignore the filter); the stripes' window test runs here, against
    // each row's date, so changing the setting needs no refetch.
    const reviewCounts = $derived(journalView.summary?.review ?? null);
    const unknownCount = $derived(reviewCounts?.unknown ?? 0);
    const recentCount = $derived(reviewCounts?.unreviewedRecent ?? 0);
    const today = localToday();
    const markerOf = (row: JournalRow): RowMarker => rowMarker(row, today, settings.reviewMaxDays, settings.reviewMarkers);
    let reviewMenuOpen = $state(false);

    /** Open a review run — the D17 keys are disabled exactly when the button is. */
    function launchReview(kind: "unknowns" | "recents"): void {
        if (!editing.canEdit || (kind === "unknowns" ? unknownCount : recentCount) === 0) return;
        reviewMenuOpen = false;
        reviewLaunch.open(kind);
    }
    // A pinned running balance (plan 36): the account the rows were fetched
    // for — the view's, not the filter bar's, so the columns appear and go
    // with the rows that fill them. Its two columns count toward `colCount`, so
    // skeleton and spacer rows span them and the layout does not jump.
    const pinned = $derived(journalView.query?.pin?.account ?? null);
    const colCount = $derived(Object.values(columns).filter(Boolean).length + (pinned === null ? 0 : 2));

    // The keyboard cursor.
    //
    // Pure visual state, NOT DOM focus. A roving tabindex over a virtualized list
    // is a focus-loss machine: scroll the cursored row out and Svelte unmounts
    // it, the browser drops focus to <body>, and the keybindings die with nothing
    // on screen to explain why. `<tr>` is also not interactive, so making it
    // focusable is exactly the shape that trips `a11y_interactive_supports_focus`
    // in a codebase with no `svelte-ignore` comments.
    const cursor = lazyListCursor<Transaction>(
        () => shown,
        (at) => slotFor(at).shows?.txn ?? null,
        (txn) => txn.index
    );
    /** The transaction `x` has armed for deletion, or null. */
    let armedDelete = $state<Transaction | null>(null);

    const announcement = $derived.by(() => {
        if (cursor.index === -1) return "";
        const txn = cursor.item;
        if (txn === null) return `Row ${cursor.index + 1} of ${shown}. Loading.`;
        return `Row ${cursor.index + 1} of ${shown}. ${txn.date}, ${txn.description || "no description"}.`;
    });

    /**
     * Bring row `position` into view.
     *
     * `nearest` for cursor moves — j and k must not throw the page around.
     * `center` for arriving from somewhere else (the problems drawer), where
     * context around the target is what you want.
     */
    function reveal(position: number, how: "center" | "nearest"): void {
        if (position < 0) return;
        const top =
            how === "center"
                ? // `topInset + headroom`, because rows begin below the sticky
                  // header in content coordinates. `centerOffset` itself is
                  // unchanged — it is simply being given the right origin now.
                  centerOffset(position, pitch, viewportHeight, topInset + headroom)
                : nearestOffset(scrollTop, position, pitch, rowHeight, viewportHeight, shown, headroom, topInset);
        if (top === scrollTop) return;
        if (scroller !== null) scroller.scrollTop = top;
        scrollTop = top;
    }

    function moveCursor(delta: number): void {
        cursor.move(delta);
        reveal(cursor.index, "nearest");
    }

    /** Open an inline editor on the cursored row. Reveal FIRST, so the row is mounted to hear it. */
    function requestRowAction(action: "description" | "category"): void {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit) return;
        reveal(cursor.index, "nearest");
        rowActions.open(txn.index, action);
    }

    async function cycleStatus(): Promise<void> {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit) return;
        const result = await editing.patch(txn.index, statusPatch(nextStatus(txn.status)));
        // The engine reassigns tindex on write, so re-anchor by position.
        cursor.reanchor();
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    /** `x` arms, a second `x` confirms. Two-step, matching the popup's delete and IfBlockCard's remove. */
    function armOrConfirmDelete(): void {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit) return;
        armedMatch = null;
        if (armedDelete !== null && armedDelete.index === txn.index) void confirmDelete();
        else armedDelete = txn;
    }

    async function confirmDelete(): Promise<void> {
        const txn = armedDelete;
        if (txn === null) return;
        armedDelete = null;
        const result = await editing.remove(txn.index);
        cursor.reanchor();
        if (!result.ok && result.failure.kind !== "conflict") editing.reportFailure(result.failure);
    }

    /** Expand or collapse the transfer row at engine `position`; anything else is left alone. */
    function toggleTransfer(position: number): void {
        const matchId = journalView.rowAt(position)?.transfer?.matchId;
        if (matchId === undefined) return;
        const open = openPositions.includes(position);
        const before = cursor.index === -1 ? null : slotFor(cursor.index);
        expanded = open ? expanded.filter((entry) => entry.position !== position) : [...expanded, {position, matchId}];
        // The cursor stays on what it showed: a half that folds away hands it
        // to its row, and a row below the toggle moves by the two halves.
        if (before === null) return;
        if (open && before.position === position) cursor.to(slotOf(position, openPositions));
        else if (before.position > position) cursor.move(open ? -2 : 2);
    }

    /** `l` on a transfer row (or one of its halves): expand or collapse it. */
    function toggleCursorTransfer(): void {
        if (cursor.index === -1) return;
        toggleTransfer(slotFor(cursor.index).position);
        reveal(cursor.index, "nearest");
    }

    // Find Matches (plan 35 D9): the badge counts the proposals, read in step
    // with the journal and the match settings.
    const matchCount = $derived(matches.count);
    /** Pairs past the lookback (A1): the button stays usable for its Show all. */
    const olderCount = $derived(matches.older);
    const canOpenMatches = $derived(matchCount > 0 || olderCount > 0);
    const noClearing = $derived(matches.clearing === false);
    let matchMenuOpen = $state(false);
    const matchTip = $derived.by(() => {
        if (noClearing) {
            return "No clearing account is declared. Tag one in your accounts file — account transfers  ; clearing: — or tick “Clearing account” in Settings › Accounts.";
        }
        const older = olderLabel(olderCount);
        if (matchCount === 0) return older === null ? "No transfer halves to pair up" : `Nothing to pair in the lookback; ${older}`;
        return older === null ? hint("Find Matches", "m") : `${hint("Find Matches", "m")} · ${older}`;
    });
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        void settings.matchWindow;
        void settings.matchLookback;
        if (serverUrl === null || generation === null || !editing.canEdit) return;
        untrack(() => void matches.sync(serverUrl, generation));
    });
    function launchMatches(): void {
        if (!editing.canEdit || !canOpenMatches) return;
        reviewMenuOpen = false;
        matchMenuOpen = false;
        matches.openRun();
    }

    // The toolbar's buttons, as the iOS app's menu (plan 42): offered exactly
    // when the buttons are usable, with the badge counts in the labels.
    registerNativeActions(() => {
        if (!editing.canEdit) return [];
        const actions: NativeAction[] = [{id: "add-transaction", label: "Add transaction", icon: "plus", run: () => txnModal.openAdd()}];
        if (unknownCount > 0)
            actions.push({
                id: "review-unknowns",
                label: `Review unknowns (${unknownCount})`,
                icon: "questionmark.circle",
                run: () => launchReview("unknowns"),
            });
        if (recentCount > 0)
            actions.push({id: "review-recents", label: `Review recents (${recentCount})`, icon: "checkmark.circle", run: () => launchReview("recents")});
        if (canOpenMatches)
            actions.push({id: "find-matches", label: matchCount > 0 ? `Find matches (${matchCount})` : "Find matches", icon: "link", run: launchMatches});
        return actions;
    });

    /** Find match for one transaction (A2): the cursored row, when it has an unlinked clearing posting. */
    const cursorFindable = (): boolean => {
        const txn = cursor.index === -1 ? null : slotFor(cursor.index).shows;
        return txn !== null && txn.transfer === undefined && findableClearing(txn.txn).length > 0;
    };
    function findMatchAtCursor(): void {
        const txn = cursor.item;
        if (txn === null || !editing.canEdit || !cursorFindable()) return;
        matches.openFor(txn.index);
    }

    // Unlink (D5) and Forget rejection (D12) are confirmed in the TOOLBAR, as
    // a delete is: it cannot unmount mid-confirm, and it can say what goes.
    type ArmedMatch = {kind: "unlink"; row: JournalRow} | {kind: "forget"; txn: Transaction; value: string};
    let armedMatch = $state.raw<ArmedMatch | null>(null);
    let matchFailure = $state<string | null>(null);

    async function confirmMatch(): Promise<void> {
        const armed = armedMatch;
        if (armed === null) return;
        armedMatch = null;
        matchFailure = null;
        const result =
            armed.kind === "unlink"
                ? armed.row.transfer === undefined
                    ? null
                    : await matches.unlink(armed.row.transfer)
                : await matches.forget(armed.txn.index, armed.value);
        if (result !== null && !result.ok && result.failure.kind !== "conflict") matchFailure = result.failure.message;
    }

    // `enabled` rather than conditional registration, so the help sheet hides
    // these on a read-only engine AND the keys stay unclaimed there.
    const canEdit = (): boolean => editing.canEdit;

    /** The list's own keys stand down while the calendar shows: they would move a cursor nobody can see. */
    const inList = (): boolean => !calendar;
    /** Keys that act on the whole journal, not a row, work in both views. */
    const BOTH_VIEWS: ReadonlySet<string> = new Set(["n", "u", "r", "m"]);

    registerKeys({
        id: "journal-table",
        priority: PRIORITY.widget,
        bindings: (
            [
                {
                    keys: "v",
                    label: "Switch between the list and the calendar",
                    group: "Journal",
                    enabled: () => calendarAllowed,
                    run: () => filters.setCalendar(!calendar),
                },
                {keys: "j", label: "Next transaction", group: "Journal", run: () => moveCursor(1)},
                {keys: "ArrowDown", label: "Next transaction", group: "Journal", run: () => moveCursor(1)},
                {keys: "k", label: "Previous transaction", group: "Journal", run: () => moveCursor(-1)},
                {keys: "ArrowUp", label: "Previous transaction", group: "Journal", run: () => moveCursor(-1)},
                // PageDown/PageUp are bound alongside the Ctrl chords deliberately:
                // Ctrl-U is view-source in Chrome and not reliably preventable, so
                // there has to be an uncontested path. Harmless in the WKWebView the
                // app ships in, real in `just dev`.
                {keys: "ctrl+d", label: "Half page down", group: "Journal", run: () => moveCursor(halfPageRows(viewportHeight, pitch))},
                {keys: "PageDown", label: "Half page down", group: "Journal", run: () => moveCursor(halfPageRows(viewportHeight, pitch))},
                {keys: "ctrl+u", label: "Half page up", group: "Journal", run: () => moveCursor(-halfPageRows(viewportHeight, pitch))},
                {keys: "PageUp", label: "Half page up", group: "Journal", run: () => moveCursor(-halfPageRows(viewportHeight, pitch))},
                {keys: "g g", label: "First transaction", group: "Journal", run: () => (cursor.first(), reveal(cursor.index, "nearest"))},
                {keys: "G", label: "Last transaction", group: "Journal", run: () => (cursor.last(), reveal(cursor.index, "nearest"))},
                {
                    keys: "Escape",
                    label: "Clear the cursor",
                    group: "Journal",
                    run: () => (armedDelete !== null ? (armedDelete = null) : armedMatch !== null ? (armedMatch = null) : cursor.clear()),
                },
                {
                    keys: "Enter",
                    label: "Edit transaction",
                    group: "Journal",
                    enabled: canEdit,
                    run: () => cursor.item !== null && txnModal.openEdit(cursor.item.index),
                },
                {keys: "e", label: "Edit description", group: "Journal", enabled: canEdit, run: () => requestRowAction("description")},
                {keys: "c", label: "Edit category", group: "Journal", enabled: canEdit, run: () => requestRowAction("category")},
                {keys: "s", label: "Cycle status", group: "Journal", enabled: canEdit, run: () => void cycleStatus()},
                {keys: "x", label: "Delete transaction", group: "Journal", enabled: canEdit, run: armOrConfirmDelete},
                {keys: "n", label: "Add transaction", group: "Journal", enabled: canEdit, run: () => txnModal.openAdd()},
                {keys: "u", label: "Review unknowns", group: "Journal", enabled: () => canEdit() && unknownCount > 0, run: () => launchReview("unknowns")},
                {keys: "r", label: "Review recents", group: "Journal", enabled: () => canEdit() && recentCount > 0, run: () => launchReview("recents")},
                {keys: "m", label: "Find matches", group: "Journal", enabled: () => canEdit() && canOpenMatches, run: launchMatches},
                {keys: "M", label: "Find a match for this transaction", group: "Journal", enabled: () => canEdit() && cursorFindable(), run: findMatchAtCursor},
                {
                    keys: "l",
                    label: "Expand or collapse a transfer",
                    group: "Journal",
                    enabled: () => cursor.index !== -1 && slotFor(cursor.index).row?.transfer !== undefined,
                    run: toggleCursorTransfer,
                },
            ] satisfies Binding[]
        ).map((binding): Binding => {
            if (binding.keys === "v" || BOTH_VIEWS.has(binding.keys)) return binding;
            const enabled = binding.enabled;
            return {...binding, enabled: enabled === undefined ? inList : () => inList() && enabled()};
        }),
    });

    /** Keep the cursor in step with the mouse: clicking a row's button cursors that row. */
    function syncCursorFromTarget(target: EventTarget | null): void {
        const row = (target as HTMLElement | null)?.closest?.("[data-slot]");
        if (row === null || row === undefined) return;
        const at = Number(row.getAttribute("data-slot"));
        if (Number.isSafeInteger(at)) cursor.to(at);
    }

    // EFFECT 1 — when the FILTER changes, jump back to the top: it is a
    // different list now, so the old scroll position means nothing.
    //
    // Keyed on `filters.value`, NOT on the rows. The rows are re-read after
    // every refresh — including the one `editing.patch` fires on success — so
    // keying on them sent the user back to row 0 after every inline edit. Nobody
    // noticed because clicking is slow; it becomes the first complaint the
    // moment a keystroke can cycle a status. `filters.value` is replaced
    // wholesale on every filter change, so its identity is a valid key.
    $effect(() => {
        void filters.value;
        if (scroller !== null) scroller.scrollTop = 0;
        scrollTop = 0;
        cursor.clear();
        armedDelete = null;
        armedMatch = null;
        expanded = [];
    });

    // WP-08: problems-drawer navigation. Scroll a txn's row into (centered)
    // view and pulse it briefly so the eye lands on the right record.
    let pulseIndex = $state<number | null>(null);
    let pulseTimer: ReturnType<typeof setTimeout> | undefined;

    /**
     * Scroll to the transaction with tindex `index`, wherever it sits in the
     * filtered view. The client holds only the rows near the viewport, so the
     * engine says where it is (`/api/journal/position`); a transaction the
     * filter hides is not jumped to.
     */
    export async function scrollToTxn(index: number): Promise<void> {
        const found = await journalView.locate(index).catch(() => null);
        if (calendar) {
            // The calendar has no scroll to move: it turns to the transaction's
            // month instead, and pulses it there.
            if (found === null || found.date === null) return;
            if (monthOf(found.date) !== monthOf(filters.value.from ?? found.date)) filters.showMonth(monthOf(found.date));
            pulse(index);
            return;
        }
        const position = found === null || found.position === null ? null : slotOf(found.position, openPositions);
        if (position === null) return;
        reveal(position, "center");
        // Arriving from the drawer leaves you on that row, so `j` continues from
        // there rather than starting over at the top.
        cursor.to(position);
        pulse(index);
    }

    function pulse(index: number): void {
        pulseIndex = index;
        clearTimeout(pulseTimer);
        pulseTimer = setTimeout(() => (pulseIndex = null), 2000);
    }

    // EFFECT 2 — re-reveal the cursor when the pitch changes.
    //
    // Pitch goes 40 → 104 across the card breakpoint, and doubles from 100 % to
    // 200 % interface scale, so the same `scrollTop` afterwards points at a
    // completely different row. That is a pre-existing bug the cursor happens
    // to make fixable: with a cursor there is finally a "here" to return to.
    $effect(() => {
        void pitch;
        reveal(
            untrack(() => cursor.index),
            "nearest"
        );
    });

    // EFFECT 3 — consume focus requests from the problems store (declared AFTER
    // the two effects above so a drawer jump wins when they land in one flush).
    $effect(() => {
        const request = problems.focusRequest;
        if (request === null) return;
        void scrollToTxn(request.txnIndex);
        problems.clearFocus();
    });

    // EFFECT 4 — a review run closed: the cursor lands on the last transaction
    // it saved (plan 32 Phase 3 "Focus"), so `j` carries on from there.
    $effect(() => {
        const request = reviewLaunch.returnTo;
        if (request === null) return;
        void scrollToTxn(request.txnIndex);
        reviewLaunch.clearReturn();
    });

    // The review settings move the badges' counts: re-read the summary (not
    // on mount — the view's own load already asked with them).
    let reviewSettingsSeen = `${settings.reviewRecentLimit}:${settings.reviewMaxDays}`;
    $effect(() => {
        const seen = `${settings.reviewRecentLimit}:${settings.reviewMaxDays}`;
        if (seen === reviewSettingsSeen) return;
        reviewSettingsSeen = seen;
        untrack(() => journalView.refreshSummary());
    });
</script>

{#snippet row(slot: Slot, shows: JournalRow, mode: "row" | "card")}
    <TransactionRow
        txn={shows.txn}
        {columns}
        {mode}
        flags={shows.flags}
        pulse={pulseIndex === shows.txn.index}
        cursor={cursor.index === slot.at}
        marker={markerOf(shows)}
        {pinned}
        running={shows.running ?? null}
        at={slot.at}
        transfer={slot.half === null ? shows.transfer : undefined}
        expanded={openPositions.includes(slot.position)}
        nested={slot.half !== null}
        linked={slot.half !== null || shows.linked === true}
        onToggle={() => toggleTransfer(slot.position)}
        onUnlink={() => ((armedDelete = null), (armedMatch = {kind: "unlink", row: shows}))}
        onForget={(value) => ((armedDelete = null), (armedMatch = {kind: "forget", txn: shows.txn, value}))}
        onFindMatch={() => matches.openFor(shows.txn.index)}
    />
{/snippet}

<!-- `min-h-40`, not `min-h-0`: on a window short in rem (a big interface
     scale, or the insights panel open on a small screen) the table keeps a few
     rows and the page scrolls, rather than squeezing to nothing and leaving its
     toolbar to spill under the totals footer. -->
<!-- The calendar's weeks are a fixed height, so in it the section is as tall as
     its content (the page scrolls) rather than a share of the window. -->
<section class={["flex grow flex-col", calendar ? "shrink-0" : "min-h-40"]} bind:clientWidth={null, (width) => (sectionWidth = width ?? null)}>
    <!-- Wraps rather than overflowing when a big interface scale leaves too few rem across. -->
    <div class="flex flex-wrap items-center justify-between gap-2 pb-1">
        {#if editing.canEdit}
            <div class="flex flex-wrap items-center gap-2">
                <button type="button" class="btn gap-1 btn-primary btn-sm" onclick={() => txnModal.openAdd()}>
                    <span class="text-base leading-none">+</span> Add transaction
                </button>
                <!-- Review (plan 32 D18): a count badge on each, disabled at zero. -->
                <span
                    class="tooltip tooltip-bottom"
                    data-tip={unknownCount === 0 ? "No uncategorised transactions in the review window" : hint("Review Unknowns", "u")}
                >
                    <button
                        type="button"
                        class="{REVIEW_BUTTON} gap-1.5"
                        disabled={unknownCount === 0}
                        onclick={() => launchReview("unknowns")}
                        data-testid="review-unknowns-button"
                    >
                        Review Unknowns
                        <span class={["badge badge-sm tabular-nums", unknownCount > 0 ? "badge-warning" : "badge-ghost"]} data-testid="review-unknowns-count"
                            >{unknownCount}</span
                        >
                    </button>
                </span>
                <div class="flex">
                    <span class="tooltip tooltip-bottom" data-tip={recentCount === 0 ? "Nothing unreviewed in the review window" : hint("Review Recents", "r")}>
                        <button
                            type="button"
                            class="{REVIEW_BUTTON} gap-1.5 rounded-r-none"
                            disabled={recentCount === 0}
                            onclick={() => launchReview("recents")}
                            data-testid="review-recents-button"
                        >
                            Review Recents
                            <span class="badge badge-ghost badge-sm tabular-nums" data-testid="review-recents-count">{recentCount}</span>
                        </button>
                    </span>
                    <!-- End-aligned when the toolbar is narrow enough to wrap: a left-aligned panel ran past the window. -->
                    <details
                        class="dropdown dropdown-end sm:dropdown-start"
                        bind:open={reviewMenuOpen}
                        use:dismissible={{active: reviewMenuOpen, outside: true, onDismiss: () => (reviewMenuOpen = false)}}
                    >
                        <summary class="{REVIEW_BUTTON} rounded-l-none border-l-base-content/20 px-1.5" aria-label="Review settings" title="Review settings">
                            <ReviewIcon name="chevron-down" class="h-3.5 w-3.5" />
                        </summary>
                        <div class="dropdown-content z-20 mt-1 rounded-box border border-base-300 bg-base-200 p-3 shadow-lg">
                            <ReviewSettings />
                        </div>
                    </details>
                </div>
                <!-- Find Matches (plan 35 D9): disabled with nothing to propose, and,
                     with no clearing account declared (D2), says how to declare one.
                     Its ▾ (Amendment A1) holds the window and the lookback, as
                     Review Recents' holds the review settings. -->
                <div class="flex">
                    <span class="tooltip tooltip-bottom before:max-w-72 before:whitespace-normal" data-tip={matchTip}>
                        <button
                            type="button"
                            class="{REVIEW_BUTTON} gap-1.5 rounded-r-none"
                            disabled={!canOpenMatches}
                            onclick={launchMatches}
                            data-testid="find-matches-button"
                        >
                            <ReviewIcon name="link" class="h-3.5 w-3.5" />
                            Find Matches
                            <span class={["badge badge-sm tabular-nums", matchCount > 0 ? "badge-info" : "badge-ghost"]} data-testid="find-matches-count"
                                >{matchCount}</span
                            >
                        </button>
                    </span>
                    <!-- End-aligned when the toolbar is narrow enough to wrap: a left-aligned panel ran past the window. -->
                    <details
                        class="dropdown dropdown-end sm:dropdown-start"
                        bind:open={matchMenuOpen}
                        use:dismissible={{active: matchMenuOpen, outside: true, onDismiss: () => (matchMenuOpen = false)}}
                    >
                        <summary
                            class="{REVIEW_BUTTON} rounded-l-none border-l-base-content/20 px-1.5"
                            aria-label="Find Matches settings"
                            title="Find Matches settings"
                            data-testid="find-matches-menu"
                        >
                            <ReviewIcon name="chevron-down" class="h-3.5 w-3.5" />
                        </summary>
                        <div class="dropdown-content z-20 mt-1 rounded-box border border-base-300 bg-base-200 p-3 shadow-lg">
                            <ReviewSettings show="matches" />
                            {#if olderCount > 0 && settings.matchLookback !== 0}
                                <p class="mt-2 text-xs text-base-content/60" data-testid="find-matches-older">{olderLabel(olderCount)}</p>
                            {/if}
                        </div>
                    </details>
                </div>
                {#if noClearing}
                    <a
                        class="link text-xs text-base-content/60 link-hover"
                        href={MATCHING_DOCS}
                        target="_blank"
                        rel="noopener noreferrer"
                        data-testid="find-matches-docs">How?</a
                    >
                {/if}
            </div>
        {:else}
            <span></span>
        {/if}
        <!-- The delete confirm lives in the TOOLBAR, not in the row: this is
             outside the virtualized area so it cannot unmount mid-confirm, and
             unlike a row-local "Delete?" it can say WHAT is about to go. It also
             gives keyboard delete a mouse path for free — there was no delete
             affordance outside the transaction popup at all. -->
        {#if armedDelete !== null}
            <div class="flex items-center gap-2 rounded border border-error/40 px-2 py-1 text-sm" role="alert">
                <span>
                    Delete <span class="font-mono">{armedDelete.date}</span>
                    {armedDelete.description || "(no description)"}?
                </span>
                <button type="button" class="btn btn-error btn-xs" onclick={() => void confirmDelete()}>Delete</button>
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (armedDelete = null)}>Keep</button>
            </div>
        {:else if armedMatch !== null}
            <div class="flex items-center gap-2 rounded border border-base-content/20 px-2 py-1 text-sm" role="alert" data-testid="match-confirm">
                {#if armedMatch.kind === "unlink"}
                    {@const transfer = armedMatch.row.transfer}
                    <span>
                        Unlink <span class="font-mono">{armedMatch.row.txn.date}</span>
                        {transfer?.from.label ?? "?"} → {transfer?.to.label ?? "?"}? Both halves stay; only their match tags go.
                    </span>
                    <button type="button" class="btn btn-warning btn-xs" onclick={() => void confirmMatch()} data-testid="match-confirm-go">Unlink</button>
                {:else}
                    <span>Forget that {armedMatch.txn.description || "this transaction"} is not a match ({armedMatch.value})? It may be proposed again.</span>
                    <button type="button" class="btn btn-warning btn-xs" onclick={() => void confirmMatch()} data-testid="match-confirm-go">Forget</button>
                {/if}
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (armedMatch = null)}>Keep</button>
            </div>
        {:else if matchFailure !== null}
            <div class="flex items-center gap-2 rounded border border-error/40 px-2 py-1 text-sm" role="alert" data-testid="match-failure">
                <span>Not saved: {matchFailure}</span>
                <button type="button" class="btn btn-ghost btn-xs" onclick={() => (matchFailure = null)}>Dismiss</button>
            </div>
        {/if}
        <div class="flex items-center gap-1">
            {#if !calendar}
                <ColumnMenu />
            {/if}
            <!-- The view toggle sits last, so hiding the column menu in the calendar does not move it. -->
            {#if calendarAllowed}
                <div class="join" role="group" aria-label="Journal view">
                    <button
                        type="button"
                        class={["btn join-item btn-ghost btn-xs", !calendar && "btn-active"]}
                        aria-pressed={!calendar}
                        aria-label="List view"
                        title={hint("List view", "v")}
                        onclick={() => filters.setCalendar(false)}
                        data-testid="view-list"
                    >
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="h-4 w-4" aria-hidden="true">
                            <path
                                fill-rule="evenodd"
                                d="M2 3.75A.75.75 0 0 1 2.75 3h10.5a.75.75 0 0 1 0 1.5H2.75A.75.75 0 0 1 2 3.75ZM2 8a.75.75 0 0 1 .75-.75h10.5a.75.75 0 0 1 0 1.5H2.75A.75.75 0 0 1 2 8Zm0 4.25a.75.75 0 0 1 .75-.75h10.5a.75.75 0 0 1 0 1.5H2.75a.75.75 0 0 1-.75-.75Z"
                                clip-rule="evenodd"
                            />
                        </svg>
                    </button>
                    <button
                        type="button"
                        class={["btn join-item btn-ghost btn-xs", calendar && "btn-active"]}
                        aria-pressed={calendar}
                        aria-label="Calendar view"
                        title={hint("Calendar view", "v")}
                        onclick={() => filters.setCalendar(true)}
                        data-testid="view-calendar"
                    >
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="h-4 w-4" aria-hidden="true">
                            <path
                                fill-rule="evenodd"
                                d="M4 1.75a.75.75 0 0 1 1.5 0V3h5V1.75a.75.75 0 0 1 1.5 0V3a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2V1.75ZM4.5 6a1 1 0 0 0-1 1v4.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V7a1 1 0 0 0-1-1h-7Z"
                                clip-rule="evenodd"
                            />
                        </svg>
                    </button>
                </div>
            {/if}
        </div>
    </div>
    <!-- There is no DOM focus on the cursored row (see the cursor's comment), so
         nothing would otherwise be announced as it moves. -->
    <div class="sr-only" aria-live="polite" aria-atomic="true">{announcement}</div>
    {#if calendar}
        <CalendarView pulse={pulseIndex} />
    {:else}
        <!-- Below `md` the page scrolls (routes/+page.svelte), so the list cannot
             take "what is left": it is one screen tall less the totals footer, and
             scrolling the page down to it leaves the list and the footer filling
             the window. The windowing needs a bounded height either way. -->
        <div
            bind:this={scroller}
            bind:clientHeight={viewportHeight}
            bind:clientWidth={containerWidth}
            onscroll={(event) => (scrollTop = event.currentTarget.scrollTop)}
            onfocusin={(event) => syncCursorFromTarget(event.target)}
            class="h-[calc(100dvh-9rem-var(--ll-chrome-top))] min-h-64 grow overflow-y-auto rounded-lg border border-base-300 md:h-auto md:min-h-0"
        >
            {#if journalView.total === null}
                <div class="flex justify-center p-8" aria-label="Loading transactions"><span class="loading loading-spinner"></span></div>
            {:else if total === 0}
                <div class="p-8 text-center text-sm text-base-content/60">No transactions match the current filters.</div>
            {:else if mode === "table"}
                <table class="table table-fixed table-sm">
                    <colgroup>
                        {#if columns.date}<col class="w-24" />{/if}
                        {#if columns.status}<col class="w-16" />{/if}
                        {#if columns.description}<col />{/if}
                        {#if columns.accounts}<col />{/if}
                        {#if columns.amount}<col class="w-36" />{/if}
                        {#if pinned !== null}<col class="w-32" /><col class="w-44" />{/if}
                    </colgroup>
                    <thead class="sticky top-0 z-10 bg-base-200" bind:clientHeight={headHeight}>
                        <tr class="text-base-content/70">
                            {#if columns.date}<th class="text-left">Date</th>{/if}
                            {#if columns.status}<th class="text-left">Status</th>{/if}
                            {#if columns.description}<th class="text-left">Description</th>{/if}
                            <!-- One measurement for the whole column: `table-fixed`
                             makes every accounts cell this wide, so the chips
                             below fit themselves to real pixels without a
                             ResizeObserver per row. -->
                            {#if columns.accounts}<th class="text-left" use:measureAccountColumn>Accounts</th>{/if}
                            {#if columns.amount}<th class="text-right">Amount</th>{/if}
                            {#if pinned !== null}
                                <th class="text-right" title="What the row did to {pinned}">Change</th>
                                <th class="text-right" title="{pinned}'s balance after the row, over the whole journal">Balance</th>
                            {/if}
                        </tr>
                    </thead>
                    <tbody>
                        {#if win.padTop > 0}
                            <tr aria-hidden="true" style="height: {win.padTop}px"><td colspan={colCount} class="p-0"></td></tr>
                        {/if}
                        {#each visible as slot (slotKey(slot))}
                            {#if slot.shows === null}
                                <!-- A row whose block is still loading: same height, no
                                 `data-txn` (nothing to act on), hidden from AT. -->
                                <tr class="h-10" aria-hidden="true" data-skeleton>
                                    <td colspan={colCount} class="py-1"><div class="h-3 w-full skeleton opacity-40"></div></td>
                                </tr>
                            {:else}
                                {@render row(slot, slot.shows, "row")}
                            {/if}
                        {/each}
                        {#if win.padBottom > 0}
                            <tr aria-hidden="true" style="height: {win.padBottom}px"><td colspan={colCount} class="p-0"></td></tr>
                        {/if}
                    </tbody>
                </table>
            {:else}
                <div class="p-2" style="padding-top: {win.padTop + topInset}px; padding-bottom: {win.padBottom + topInset}px">
                    {#each visible as slot (slotKey(slot))}
                        {#if slot.shows === null}
                            <div class="mb-2 h-24 skeleton opacity-40" aria-hidden="true" data-skeleton></div>
                        {:else}
                            {@render row(slot, slot.shows, "card")}
                        {/if}
                    {/each}
                </div>
            {/if}
        </div>
    {/if}
</section>
