<script lang="ts">
    // The Accrual section of the transaction editor (plan 45 D-editor), built
    // like SimilarSection: a collapsible block below the fields, single mode
    // only, and only while the helpers are on (`accrual.enabled`). What it
    // shows follows the card's postings, re-read on every edit
    // (`classifyCard`): a deferral to schedule, a schedule's summary with
    // Edit and End contract, "earned at once or over a term", an invoice's
    // status, a payment's invoice candidates, a credit note's contract.
    //
    // A schedule the user sets up writes its source tags onto the card's
    // posting at once (`recognition: ID`, `term-months`, …), so the editor's
    // normal save writes the transaction with them; the overlay then writes the
    // rules (`pending`) — journal first, rules last (D-editor, Risks). The
    // previews come from the engine and are only displayed here.
    import {untrack} from "svelte";
    import {addComment, formToBody} from "#lib/api/editMapping.js";
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import type {Transaction} from "#lib/domain/types.js";
    import {fmt} from "#lib/format/amounts.js";
    import {add, neg, type Dec} from "#lib/domain/money.js";
    import type {ReviewForm} from "#lib/review/types.js";
    import {reviewLaunch} from "#lib/review/ui/launch.svelte.js";
    import ReviewIcon from "#lib/review/ui/ReviewIcon.svelte";
    import {editing} from "#lib/stores/editing.svelte.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import {settings} from "#lib/stores/settings.svelte.js";
    import {
        CUSTOMER_TAG,
        dayShort,
        draftToWire,
        monthShort,
        newScheduleId,
        partyKindFor,
        specToDraft,
        termLabel,
        VENDOR_TAG,
        type ScheduleDraft,
    } from "../model";
    import {accrual} from "../store.svelte";
    import type {ContractKind, Restatement, Schedule, TransactionSchedules} from "../types";
    import {
        accountsWithRole,
        applyDrafts,
        classifyCard,
        declarationHint,
        defaultTarget,
        deferralRoleFor,
        draftProblem,
        freshDraft,
        idsTaken,
        linesFromCard,
        scheduleLetter,
        summarize,
        type AccrualLine,
    } from "./editorModel";
    import RestatementPreview from "./RestatementPreview.svelte";
    import ScheduleForm from "./ScheduleForm.svelte";
    import SchedulePreview from "./SchedulePreview.svelte";
    import SettlementPicker from "./SettlementPicker.svelte";

    type Mode = "new" | "edit" | "end";
    interface Entry {
        draft: ScheduleDraft;
        /** The card's postings carrying this schedule's tags (none for a credit note's End contract). */
        postings: number[];
        mode: Mode;
        /** "Prefilled from …" when a renewal prefill applied. */
        banner: string | null;
    }
    interface Preview {
        key: string;
        schedule: Schedule | null;
        restatement: Restatement | null;
        error: string | null;
    }

    let {
        form = $bindable(),
        txn,
        fingerprint,
        adding,
        today,
        classes,
        accountNames,
        disabled = false,
        pending = $bindable([]),
        blocking = $bindable(null),
        onApply,
        onReload,
    }: {
        form: ReviewForm;
        /** The card's transaction as loaded (a blank one when adding). */
        txn: Transaction;
        fingerprint: string | null;
        adding: boolean;
        today: string;
        classes: AccountClasses;
        accountNames: readonly string[];
        disabled?: boolean;
        /** The schedules to write after the transaction saves. */
        pending?: ScheduleDraft[];
        /** Why the schedules can't be saved yet ("Schedule A: choose the account…"), or null. */
        blocking?: string | null;
        /** Apply all (a restatement's preview): the editor's save. */
        onApply?: () => void;
        /** A link was written: the editor reloads the transaction. */
        onReload?: () => void;
    } = $props();

    let entries = $state<Entry[]>([]);
    /** Each posting's comment before the section first tagged it, to restore on Remove. */
    let baselines = $state<Record<number, string>>({});
    let forData = $state.raw<TransactionSchedules | null>(null);
    let previews = $state<Record<string, Preview>>({});
    let open = $state<boolean | null>(null);
    let earnedPick = $state<Record<number, string>>({});
    let clearCandidate = $state(false);

    // The status (gate, roles) may not have been read on this page yet.
    $effect(() => {
        const serverUrl = settings.serverUrl;
        const generation = journal.generation;
        if (serverUrl === null || generation === null || !editing.canEdit) return;
        untrack(() => void accrual.sync(serverUrl, generation));
    });

    // The transaction's schedules, renewal prefill and party suggestion (edit only).
    $effect(() => {
        const index = txn.index;
        const enabled = accrual.enabled;
        void fingerprint;
        if (adding || !enabled || index <= 0) return;
        let live = true;
        accrual.forTransaction(index).then(
            (found) => {
                if (live) forData = found;
            },
            () => {
                if (live) forData = null;
            }
        );
        return () => {
            live = false;
        };
    });

    const draftIds = $derived(new Set(entries.filter((entry) => entry.mode === "new").map((entry) => entry.draft.id)));
    const lines = $derived(accrual.enabled ? classifyCard(form, (account) => accrual.roleOf(account), classes, txn.tags) : []);
    /** A line our own new draft tagged still reads as the deferral it was. */
    const shown = $derived(
        lines.map((line) => (line.kind === "scheduled" && line.id !== null && draftIds.has(line.id) ? {...line, kind: "unscheduled" as const} : line))
    );
    const inEntry = (posting: number): Entry | undefined => entries.find((entry) => entry.postings.includes(posting));
    const unscheduled = $derived(shown.filter((line) => line.kind === "unscheduled" && inEntry(line.posting) === undefined));
    const scheduled = $derived(shown.filter((line) => line.kind === "scheduled" && inEntry(line.posting) === undefined));
    const earned = $derived(shown.filter((line) => line.kind === "earned"));
    const invoices = $derived(shown.filter((line) => line.kind === "invoice"));
    const payments = $derived(shown.filter((line) => line.kind === "payment"));
    const credits = $derived(shown.filter((line) => line.kind === "credit"));
    const visible = $derived(accrual.enabled && (shown.length > 0 || entries.length > 0));
    const inferredContract = $derived<ContractKind>(forData?.renewal != null ? "renewal" : "new");
    const partyNames = $derived(
        reviewLaunch.tags
            .filter((tag) => tag.name === CUSTOMER_TAG || tag.name === VENDOR_TAG)
            .flatMap((tag) => tag.values.map((value) => value.value))
            .filter((name, at, all) => all.indexOf(name) === at)
    );

    // Opens for what needs the user (D-editor "Opens"): a deferral to
    // schedule, a credit note, a payment with a clear candidate. Once.
    $effect(() => {
        if (open !== null || !visible) return;
        const wanted = unscheduled.length > 0 || credits.length > 0 || clearCandidate;
        untrack(() => (open = wanted ? true : null));
        if (wanted) return;
        if (scheduled.length + earned.length + invoices.length + payments.length > 0) untrack(() => (open = false));
    });

    // What the overlay writes after the transaction, and what holds it back.
    $effect(() => {
        const drafts = entries.map((entry) => entry.draft);
        const problem = entries
            .map((entry, at) => {
                const why = draftProblem(entry.draft);
                return why === null ? null : `${scheduleLetter(at)}: ${why}`;
            })
            .find((why) => why !== null);
        untrack(() => {
            pending = drafts.map((draft) => ({...draft, lines: draft.lines.map((line) => ({...line}))}));
            blocking = problem ?? null;
        });
    });

    function money(line: AccrualLine): string {
        if (line.amount === null) return "";
        return fmt(line.commodity, line.amount.m < 0n ? neg(line.amount) : line.amount, journal.reportStyles);
    }

    function rememberBaseline(postings: readonly number[]): void {
        const next = {...baselines};
        for (const at of postings) if (next[at] === undefined) next[at] = form.postings[at]?.comment ?? "";
        baselines = next;
    }

    /** Write every entry's tags onto the card. */
    function tagCard(): void {
        form = applyDrafts(
            form,
            entries.map((entry) => ({draft: entry.draft, postings: entry.postings}))
        );
    }

    function takenIds(): ReadonlySet<string> {
        return idsTaken(
            entries.map((entry) => entry.draft.id),
            lines.map((line) => line.id),
            (forData?.schedules ?? []).map((schedule) => schedule.spec.id)
        );
    }

    function prefillBanner(): {banner: string; draft: Partial<ScheduleDraft>} | null {
        const renewal = forData?.renewal ?? null;
        if (renewal === null) return null;
        const spec = renewal.prefill;
        const party = spec.lines.find((line) => line.party !== null)?.party ?? null;
        return {
            banner: `Prefilled from ${party?.name ?? "the previous contract"} · ${monthShort(spec.start.slice(0, 7))} · ${termLabel(spec.term)} · ${spec.id}`,
            draft: {term: spec.term, proration: spec.proration, contract: renewal.contract},
        };
    }

    /** "Recognize over time" for `line`: a new schedule over it. */
    function startSchedule(line: AccrualLine): void {
        const prefill = prefillBanner();
        const target = defaultTarget(form, line, classes, forData?.renewal?.prefill.lines[0]?.targetAccount ?? null);
        const base = freshDraft(newScheduleId(takenIds()), form, [line], [target], line.party, inferredContract);
        const draft: ScheduleDraft = prefill === null ? base : {...base, ...prefill.draft};
        rememberBaseline([line.posting]);
        entries = [...entries, {draft, postings: [line.posting], mode: "new", banner: prefill?.banner ?? null}];
        open = true;
        tagCard();
    }

    /** Put `line` on an existing new schedule (same terms: one schedule, D-editor "one per distinct terms"). */
    function joinSchedule(line: AccrualLine, at: number): void {
        const entry = entries[at];
        if (entry === undefined) return;
        rememberBaseline([line.posting]);
        const postings = [...entry.postings, line.posting];
        const cardLines = postings.map((posting) => lines.find((candidate) => candidate.posting === posting)).filter((found) => found !== undefined);
        entries = entries.map((candidate, i) => (i === at ? {...candidate, postings, draft: linesFromCard(candidate.draft, cardLines)} : candidate));
        tagCard();
    }

    /** Edit schedule / End contract on an existing one. */
    function editSchedule(schedule: Schedule, line: AccrualLine | null, mode: "edit" | "end"): void {
        const contract = (line === null ? null : (form.postings[line.posting]?.comment.match(/contract:\s*([a-z]+)/)?.[1] ?? null)) as ContractKind | null;
        const base = specToDraft(schedule.spec, txn.date, contract);
        const draft: ScheduleDraft = mode === "end" ? {...base, term: {through: today}} : base;
        const postings = line === null ? [] : lines.filter((candidate) => candidate.id === schedule.spec.id).map((candidate) => candidate.posting);
        rememberBaseline(postings);
        entries = [...entries, {draft, postings, mode, banner: null}];
        open = true;
        tagCard();
    }

    function updateDraft(at: number, draft: ScheduleDraft): void {
        entries = entries.map((entry, i) => (i === at ? {...entry, draft} : entry));
        tagCard();
    }

    function removeEntry(at: number): void {
        const entry = entries[at];
        if (entry === undefined) return;
        entries = entries.filter((_, i) => i !== at);
        const postings = form.postings.map((posting, i) =>
            entry.postings.includes(i) && baselines[i] !== undefined ? {...posting, comment: baselines[i]} : posting
        );
        form = {...form, postings};
        tagCard();
    }

    /** "Earned over a term": the line moves to the deferral account and gets a schedule recognizing into where it was. */
    function overATerm(line: AccrualLine, deferral: string): void {
        const target = line.account;
        rememberBaseline([line.posting]);
        form = {...form, postings: form.postings.map((posting, i) => (i === line.posting ? {...posting, account: deferral} : posting))};
        const role = deferralRoleFor(line, classes);
        const moved: AccrualLine = {...line, account: deferral, role, kind: "unscheduled"};
        const prefill = prefillBanner();
        const base = freshDraft(newScheduleId(takenIds()), form, [moved], [target], line.party, inferredContract);
        const draft: ScheduleDraft = prefill === null ? base : {...base, ...prefill.draft, lines: base.lines};
        entries = [...entries, {draft, postings: [line.posting], mode: "new", banner: prefill?.banner ?? null}];
        open = true;
        tagCard();
    }

    // The live previews (debounced): a new schedule by its spec alone; a
    // restatement against the edited source, so it sees the new amounts.
    let previewTimer: ReturnType<typeof setTimeout> | undefined;
    $effect(() => {
        const snapshot = entries.map((entry) => ({draft: $state.snapshot(entry.draft) as ScheduleDraft, mode: entry.mode}));
        const body = JSON.stringify(form.postings.map((posting) => [posting.account, posting.amount, posting.commodity]));
        clearTimeout(previewTimer);
        previewTimer = setTimeout(() => untrack(() => void loadPreviews(snapshot, body)), 300);
        return () => clearTimeout(previewTimer);
    });

    async function loadPreviews(snapshot: {draft: ScheduleDraft; mode: Mode}[], body: string): Promise<void> {
        for (const {draft, mode} of snapshot) {
            const key = JSON.stringify([draftToWire(draft), mode === "new" ? "" : body]);
            if (previews[draft.id]?.key === key) continue;
            if (draftProblem(draft) !== null) {
                previews = {...previews, [draft.id]: {key, schedule: null, restatement: null, error: null}};
                continue;
            }
            const source =
                mode !== "new" && !adding && fingerprint !== null
                    ? {
                          kind: "replace" as const,
                          index: txn.index,
                          fingerprint,
                          transaction: formToBody({...form, comment: addComment(form)}, journal.dominantCommodity),
                      }
                    : undefined;
            try {
                const plan = await accrual.preview({schedules: [draftToWire(draft)], ...(source === undefined ? {} : {source})});
                previews = {
                    ...previews,
                    [draft.id]: {
                        key,
                        schedule: plan?.appended.find((schedule) => schedule.spec.id === draft.id) ?? null,
                        restatement: plan?.restatements.find((restatement) => restatement.id === draft.id) ?? null,
                        error: null,
                    },
                };
            } catch (cause) {
                previews = {...previews, [draft.id]: {key, schedule: null, restatement: null, error: cause instanceof Error ? cause.message : String(cause)}};
            }
        }
    }

    function scheduleOf(line: AccrualLine): Schedule | null {
        return forData?.schedules.find((schedule) => schedule.spec.id === line.id) ?? null;
    }

    function summaryText(schedule: Schedule): string {
        const counts = summarize(schedule.periods, today);
        const amount = schedule.periods.find((period) => period.part === "regular") ?? schedule.periods[0];
        const each =
            amount === undefined
                ? ""
                : (() => {
                      const commodity = schedule.spec.lines[0]?.total.commodity ?? "";
                      const total = amount.amounts.reduce((sum, value) => add(sum, value), {m: 0n, p: 0} as Dec);
                      return ` × ${fmt(commodity, total.m < 0n ? neg(total) : total, journal.reportStyles)}`;
                  })();
        const parts = [`${counts.periods}${each}`, `${counts.posted} posted`];
        if (counts.due > 0) parts.push(`${counts.due} due`);
        if (counts.closed > 0) parts.push(`${counts.closed} due in closed periods`);
        if (counts.next !== null) parts.push(`next ${dayShort(counts.next)}`);
        return parts.join(" · ");
    }

    const creditContract = $derived(forData?.renewal?.prefill ?? null);
</script>

{#if visible}
    <section class="rounded-box border border-base-content/10 bg-base-200/40" data-testid="accrual-section" data-open={open === true ? "" : undefined}>
        <button
            type="button"
            class="flex w-full items-center gap-1.5 px-3 py-2 text-left text-sm font-medium"
            onclick={() => (open = open !== true)}
            aria-expanded={open === true}
        >
            <ReviewIcon name={open === true ? "chevron-down" : "chevron-right"} class="h-3.5 w-3.5" />
            Accrual
            {#if pending.length > 0}<span class="badge badge-sm badge-info" title={blocking ?? undefined}>{pending.length} to save</span>{/if}
        </button>
        {#if open === true}
            <div class="flex flex-col gap-4 border-t border-base-content/10 px-3 py-3">
                {#each unscheduled as line (line.posting)}
                    <div class="flex flex-wrap items-center justify-between gap-2 text-sm" data-testid="accrual-unscheduled">
                        <span><span class="font-mono text-xs">{line.account}</span> {money(line)} has no schedule.</span>
                        <span class="flex gap-1">
                            {#each entries as entry, at (entry.draft.id)}
                                {#if entry.mode === "new"}
                                    <button type="button" class="btn btn-ghost btn-xs" {disabled} onclick={() => joinSchedule(line, at)}
                                        >Same terms as {scheduleLetter(at)}</button
                                    >
                                {/if}
                            {/each}
                            <button type="button" class="btn btn-primary btn-xs" {disabled} onclick={() => startSchedule(line)} data-testid="accrual-recognize"
                                >Recognize over time</button
                            >
                        </span>
                    </div>
                {/each}

                {#each scheduled as line (line.posting)}
                    {@const schedule = scheduleOf(line)}
                    <div class="flex flex-col gap-1 text-sm" data-testid="accrual-scheduled">
                        <span><span class="font-mono text-xs">{line.account}</span> · {line.id}</span>
                        {#if schedule === null}
                            <span class="text-xs text-muted"
                                >{forData === null ? "Loading its schedule…" : "No rule for this schedule in the recognition file."}</span
                            >
                        {:else}
                            <span class="text-xs" data-testid="accrual-summary">{summaryText(schedule)}</span>
                            <span class="flex gap-1">
                                <button
                                    type="button"
                                    class="btn btn-ghost btn-xs"
                                    {disabled}
                                    onclick={() => editSchedule(schedule, line, "edit")}
                                    data-testid="accrual-edit-schedule">Edit schedule</button
                                >
                                <button
                                    type="button"
                                    class="btn btn-ghost btn-xs"
                                    {disabled}
                                    onclick={() => editSchedule(schedule, line, "end")}
                                    data-testid="accrual-end-contract">End contract</button
                                >
                            </span>
                        {/if}
                    </div>
                {/each}

                {#each earned as line (line.posting)}
                    {@const role = deferralRoleFor(line, classes)}
                    {@const choices = accrual.status === null ? [] : accountsWithRole(accrual.status.roles, role)}
                    <div class="flex flex-col gap-1 text-sm" data-testid="accrual-earned">
                        <span><span class="font-mono text-xs">{line.account}</span> {money(line)} is earned</span>
                        <label class="flex items-center gap-2"><input type="radio" class="radio radio-xs" checked name="earned-{line.posting}" /> at once</label
                        >
                        <label class="flex items-center gap-2">
                            <input
                                type="radio"
                                class="radio radio-xs"
                                name="earned-{line.posting}"
                                disabled={disabled || choices.length === 0}
                                onchange={() => {
                                    const pick = earnedPick[line.posting] ?? choices[0];
                                    if (pick !== undefined && choices.length === 1) overATerm(line, pick);
                                }}
                                data-testid="accrual-over-a-term"
                            />
                            over a term
                        </label>
                        {#if choices.length === 0}
                            <p class="text-xs text-muted" data-testid="accrual-declare-hint">
                                No {role === "prepaid" ? "prepaid" : "deferred-revenue"} account is declared. Add one to your accounts file:
                                <code class="block font-mono">{declarationHint(role)}</code>
                            </p>
                        {:else if choices.length > 1}
                            <span class="flex items-center gap-2">
                                <select
                                    class="select select-xs"
                                    value={earnedPick[line.posting] ?? choices[0]}
                                    onchange={(event) => (earnedPick = {...earnedPick, [line.posting]: event.currentTarget.value})}
                                >
                                    {#each choices as account (account)}<option value={account}>{account}</option>{/each}
                                </select>
                                <button
                                    type="button"
                                    class="btn btn-ghost btn-xs"
                                    {disabled}
                                    onclick={() => overATerm(line, earnedPick[line.posting] ?? choices[0] ?? "")}>Defer</button
                                >
                            </span>
                        {/if}
                    </div>
                {/each}

                {#each invoices as line (line.posting)}
                    <div class="text-sm" data-testid="accrual-invoice">
                        {line.role === "payable" ? "Bill" : "Invoice"}{line.invoice === null ? "" : ` ${line.invoice}`}
                        {#if line.party !== null}· {line.party.name}{/if}
                        · <span class={line.matched ? "text-success" : "text-warning"} data-testid="accrual-paid">{line.matched ? "Paid" : "Unpaid"}</span>
                    </div>
                {/each}

                {#each payments as line (line.posting)}
                    <div class="flex flex-col gap-1 text-sm" data-testid="accrual-payment">
                        <span>Payment {money(line)} — which invoice does it pay?</span>
                        {#if adding || txn.index <= 0}
                            <span class="text-xs text-muted">Save the transaction first to match it to an invoice.</span>
                        {:else if line.matched}
                            <span class="text-xs text-success">Linked to its invoice.</span>
                        {:else}
                            <SettlementPicker index={txn.index} {disabled} onLinked={onReload} onLoaded={(clear) => (clearCandidate = clear)} />
                        {/if}
                    </div>
                {/each}

                {#each credits as line (line.posting)}
                    <div class="flex flex-col gap-1 text-sm" data-testid="accrual-credit">
                        {#if creditContract !== null}
                            <span>Credits {line.party?.name ?? "this customer"}'s contract {creditContract.id}?</span>
                            <button
                                type="button"
                                class="btn self-start btn-ghost btn-xs"
                                {disabled}
                                onclick={() => {
                                    const draft = specToDraft(creditContract, txn.date, null);
                                    entries = [...entries, {draft: {...draft, term: {through: form.date}}, postings: [], mode: "end", banner: null}];
                                }}
                                data-testid="accrual-credit-end">End contract</button
                            >
                        {:else}
                            <span class="text-xs text-muted">A credit to {line.account} with no contract found for {line.party?.name ?? "its customer"}.</span>
                        {/if}
                    </div>
                {/each}

                {#each entries as entry, at (entry.draft.id)}
                    {@const preview = previews[entry.draft.id]}
                    {@const firstLine = lines.find((line) => entry.postings.includes(line.posting))}
                    <div class="flex flex-col gap-3 rounded border border-base-content/10 bg-base-100 p-3" data-testid="accrual-entry" data-mode={entry.mode}>
                        <ScheduleForm
                            draft={entry.draft}
                            label={entry.mode === "end" ? `End contract` : entry.mode === "edit" ? "Edit schedule" : scheduleLetter(at)}
                            partyKind={firstLine?.role != null ? partyKindFor(firstLine.role) : (entry.draft.party?.kind ?? "customer")}
                            sourceDescription={form.description}
                            {accountNames}
                            {partyNames}
                            {inferredContract}
                            partySuggestion={forData?.partySuggestion ?? null}
                            banner={entry.banner}
                            endOnly={entry.mode === "end"}
                            {disabled}
                            onChange={(draft) => updateDraft(at, draft)}
                        />
                        {#if draftProblem(entry.draft) !== null}
                            <p class="text-xs text-muted" data-testid="accrual-draft-problem">{draftProblem(entry.draft)}</p>
                        {:else if preview === undefined}
                            <span class="loading loading-xs loading-spinner" aria-label="Loading the preview"></span>
                        {:else if preview.error !== null}
                            <p class="text-xs text-error" role="alert" data-testid="accrual-preview-error">{preview.error}</p>
                        {:else if preview.restatement !== null}
                            <RestatementPreview
                                restatement={preview.restatement}
                                commodity={entry.draft.lines[0]?.commodity ?? ""}
                                busy={disabled}
                                {onApply}
                                onCancel={() => removeEntry(at)}
                            />
                        {:else if preview.schedule !== null}
                            <SchedulePreview schedule={preview.schedule} />
                        {/if}
                        <button type="button" class="btn self-end btn-ghost btn-xs" {disabled} onclick={() => removeEntry(at)} data-testid="accrual-remove"
                            >Remove</button
                        >
                    </div>
                {/each}
            </div>
        {/if}
    </section>
{/if}
