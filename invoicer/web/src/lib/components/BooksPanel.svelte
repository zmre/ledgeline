<!-- The Books panel (ARCHITECTURE §6.3), read-only: what the journal holds
     under the invoice's number. The transactions it is booked as, its
     receivable and payments, any void, and whether the file's `paid:` mirror
     says what the books say. Everything shown is the server's text. -->
<script lang="ts">
    import {paidText, type EntryView, type InvoiceBooks} from "#lib/invoices.js";

    let {books}: {books: InvoiceBooks} = $props();

    const empty = $derived(books.bookings.length === 0 && books.voids.length === 0 && books.payments.length === 0);
    const stateTone: Record<string, string> = {open: "badge-warning", partial: "badge-warning", paid: "badge-success", void: "badge-ghost"};
</script>

{#snippet entry(item: EntryView, label: string)}
    <div class="rounded-box border border-base-300 bg-base-100 p-3" data-testid="books-entry">
        <div class="flex items-baseline justify-between gap-2">
            <div class="min-w-0">
                <span class="font-medium tabular-nums">{item.date}</span>
                <span class="ml-1 truncate">{item.description}</span>
            </div>
            <span class="badge shrink-0 badge-ghost badge-xs">{label}</span>
        </div>
        <div class="mt-0.5 text-xs text-base-content/60">{item.file}:{item.line}</div>
        <table class="mt-2 w-full text-sm">
            <tbody>
                {#each item.postings as posting, i (i)}
                    <tr>
                        <td class="py-0.5 pr-2 font-mono text-xs break-all">{posting.account}</td>
                        <td class="py-0.5 text-right whitespace-nowrap tabular-nums">{posting.amounts.join(", ")}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
{/snippet}

<section class="flex flex-col gap-3" aria-labelledby="books-heading" data-testid="books-panel">
    <div class="flex items-center justify-between">
        <h2 id="books-heading" class="text-sm font-semibold tracking-wide uppercase">Books</h2>
        <span class="badge badge-outline badge-sm">{books.basis === "cash" ? "Cash basis" : "Accrual basis"}</span>
    </div>

    {#if books.receivable !== null}
        <div class="rounded-box bg-base-200 p-3" data-testid="books-receivable">
            <div class="flex items-center justify-between gap-2">
                <span class="text-sm text-base-content/70">Receivable</span>
                <span class="badge badge-sm {stateTone[books.receivable.state] ?? 'badge-ghost'}">{books.receivable.state}</span>
            </div>
            <div class="mt-1 font-mono text-xs break-all">{books.receivable.account}</div>
            <dl class="mt-2 grid grid-cols-2 gap-x-3 gap-y-1 text-sm">
                <dt class="text-base-content/70">Booked</dt>
                <dd class="text-right tabular-nums">{books.receivable.amount}</dd>
                <dt class="text-base-content/70">Outstanding</dt>
                <dd class="text-right font-semibold tabular-nums" data-testid="books-outstanding">{books.receivable.outstanding}</dd>
            </dl>
            {#if books.receivable.payments.length > 0}
                <div class="mt-2 text-xs text-base-content/70">Payments and credits</div>
                <ul class="text-sm">
                    {#each books.receivable.payments as payment, i (i)}
                        <li class="flex justify-between tabular-nums"><span>{payment.date}</span><span>{payment.amount}</span></li>
                    {/each}
                </ul>
            {/if}
        </div>
    {/if}

    <div class="text-sm" data-testid="books-mirror">
        <span class="text-base-content/70">paid:</span>
        <span class="font-medium">{paidText(books.paid)}</span>
        {#if books.in_step === false}
            <div class="mt-1 alert px-3 py-2 text-xs alert-warning" role="status">
                The books say <code>{books.books_paid_text}</code>. The mirror is rewritten when the books change.
            </div>
        {/if}
    </div>

    {#if empty}
        <p class="text-sm text-base-content/70" data-testid="books-empty">
            {#if books.status === "draft"}
                Not booked: drafts never touch the books.
            {:else if books.basis === "cash"}
                Nothing booked yet: cash books book an invoice when a payment is applied to it.
            {:else}
                Nothing in the books under this number.
            {/if}
        </p>
    {/if}

    {#each books.bookings as booking, i (i)}
        {@render entry(booking, "Booked")}
    {/each}
    {#each books.payments as payment, i (i)}
        <div>
            {@render entry(payment, `Payment ${payment.amount}`)}
        </div>
    {/each}
    {#each books.voids as reversal, i (i)}
        {@render entry(reversal, "Void")}
    {/each}

    {#if books.problems.length > 0}
        <ul class="flex flex-col gap-1" data-testid="books-problems">
            {#each books.problems as problem, i (i)}
                <li class="alert px-3 py-2 text-xs {problem.severity === 'error' ? 'alert-error' : 'alert-warning'}">{problem.message}</li>
            {/each}
        </ul>
    {/if}
</section>
