<script lang="ts">
    // D4's restatement preview: the rule before and after, every posted entry
    // with its amount before and after and what happens to it (kept in a
    // closed period, rewritten, deleted), the one catch-up adjustment, and the
    // schedule's total after. Nothing is written until the editor's Save
    // (Apply all) — the transaction first, then the entries, then the rules.
    import {fmt} from "#lib/format/amounts.js";
    import {add, neg, type Dec} from "#lib/domain/money.js";
    import {journal} from "#lib/stores/journal.svelte.js";
    import type {Restatement} from "../types";

    let {
        restatement,
        commodity,
        busy = false,
        onApply,
        onCancel,
    }: {
        restatement: Restatement;
        commodity: string;
        busy?: boolean;
        onApply?: () => void;
        onCancel?: () => void;
    } = $props();

    const sum = (values: readonly Dec[]): Dec => values.reduce((total, value) => add(total, value), {m: 0n, p: 0} as Dec);
    const money = (value: Dec): string => fmt(commodity, value.m < 0n ? neg(value) : value, journal.reportStyles);
    const ACTION = {kept: "kept", unchanged: "unchanged", rewrite: "rewrite", delete: "delete"} as const;
    const adjustment = $derived.by(() => {
        const entry = restatement.adjustment;
        if (entry === null) return null;
        const amount = entry.postings.flatMap((posting) => posting.amounts).find((amount) => amount.quantity.m > 0n) ?? null;
        const restated = entry.tags.find(([name]) => name === "restated")?.[1] ?? null;
        return {date: entry.date, restated, amount};
    });
</script>

<div class="flex flex-col gap-2 text-sm" data-testid="accrual-restatement">
    <p class="font-medium">Restate {restatement.id}</p>
    {#if restatement.ruleChange !== null}
        <div class="rounded bg-base-200 p-2 font-mono text-xs">
            <pre class="text-error" data-testid="accrual-rule-before">− {restatement.ruleChange.before}</pre>
            <pre class="text-success" data-testid="accrual-rule-after">+ {restatement.ruleChange.after}</pre>
        </div>
    {/if}
    {#if restatement.rows.length > 0}
        <table class="table table-xs">
            <thead>
                <tr><th>Posted entries</th><th class="text-right">before</th><th class="text-right">after</th><th>action</th></tr>
            </thead>
            <tbody>
                {#each restatement.rows as row (row.index)}
                    <tr data-testid="accrual-restatement-row" data-action={row.action}>
                        <td class="font-mono">{row.recognized}{row.closed ? " (closed)" : ""}</td>
                        <td class="text-right tabular-nums">{money(sum(row.before))}</td>
                        <td class="text-right tabular-nums">{row.after === null ? "—" : money(sum(row.after))}</td>
                        <td>{ACTION[row.action]}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    {/if}
    {#if adjustment !== null}
        <p data-testid="accrual-adjustment">
            Catch-up adjustment {adjustment.date}{adjustment.restated === null ? "" : ` · restated: ${adjustment.restated}`}
            {#if adjustment.amount !== null}<span class="tabular-nums">
                    · {fmt(adjustment.amount.commodity, adjustment.amount.quantity, journal.reportStyles)}</span
                >{/if}
        </p>
    {/if}
    <p class="text-xs text-base-content/70" data-testid="accrual-total-after">Schedule total after: {money(sum(restatement.totalAfter))}</p>
    {#if onApply !== undefined || onCancel !== undefined}
        <div class="flex justify-end gap-2">
            {#if onCancel !== undefined}<button type="button" class="btn btn-ghost btn-xs" disabled={busy} onclick={onCancel}>Cancel</button>{/if}
            {#if onApply !== undefined}
                <button type="button" class="btn btn-primary btn-xs" disabled={busy} onclick={onApply} data-testid="accrual-apply-all">Apply all</button>
            {/if}
        </div>
    {/if}
</div>
