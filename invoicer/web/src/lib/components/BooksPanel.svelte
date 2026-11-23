<!-- The Books panel (ARCHITECTURE §6.3), read-only: what the journal holds
     under the invoice's number, full width under the invoice.

     - one plain line on where it stands ("Paid in full on …", "Partly paid:
       $X of $Y, $Z due …", "Unpaid: … due …"), from the server's settlement;
     - where it was booked: each transaction's postings, account by account
       (receivable, revenue, deposit liability, …);
     - the payments and credits against it: date, where the money went,
       description, amount (cash books: the payment transactions, which are
       where cash books book an invoice);
     - a void's reversal;
     - the file's `paid:` line only when it disagrees with the books, in
       plain words, and anything `check` finds.

     Everything shown is the server's text (Patrick's feedback, 2026-11-23:
     the old "Payments and credits" list and the `paid:` mirror line said too
     little and repeated "paid"). -->
<script lang="ts">
    import {settlementText, type EntryView, type InvoiceBooks} from "#lib/invoices.js";

    let {books}: {books: InvoiceBooks} = $props();

    const empty = $derived(books.bookings.length === 0 && books.voids.length === 0 && books.payments.length === 0);
    const tone: Record<string, string> = {paid: "text-success", partial: "text-warning", unpaid: "text-warning", void: "text-base-content/70"};
    const received = $derived(books.receivable?.payments ?? []);
    const receivableAccount = $derived(books.receivable?.account ?? null);
</script>

{#snippet postings(item: EntryView, label: string, testid: string)}
    <div class="min-w-0" data-testid={testid}>
        <div class="flex flex-wrap items-baseline gap-x-2 text-sm">
            <span class="font-medium">{label}</span>
            <span class="tabular-nums">{item.date}</span>
            <span class="min-w-0 truncate text-base-content/80">{item.description}</span>
            <span class="ml-auto text-xs text-base-content/60" title="Where it is in the journal">{item.file}:{item.line}</span>
        </div>
        <table class="table mt-1 table-sm">
            <thead>
                <tr>
                    <th>Account</th>
                    <th class="text-right">Amount</th>
                </tr>
            </thead>
            <tbody>
                {#each item.postings as posting, i (i)}
                    <tr data-testid="books-posting">
                        <td class="font-mono text-xs break-all">
                            {posting.account}
                            {#if posting.account === receivableAccount && testid === "books-booking"}
                                <span class="ml-1 badge badge-ghost font-sans badge-xs">receivable</span>
                            {/if}
                        </td>
                        <td class="text-right whitespace-nowrap tabular-nums">{posting.amounts.join(", ")}</td>
                    </tr>
                {/each}
            </tbody>
        </table>
    </div>
{/snippet}

<section class="flex flex-col gap-4 rounded-box border border-base-300 p-4" aria-labelledby="books-heading" data-testid="books-panel">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
        <h2 id="books-heading" class="text-sm font-semibold tracking-wide uppercase">Books</h2>
        <span class="badge badge-outline badge-sm">{books.basis === "cash" ? "Cash basis" : "Accrual basis"}</span>
        {#if books.settlement !== null}
            <span class="font-medium {tone[books.settlement.state] ?? ''}" data-testid="books-status">{settlementText(books.settlement)}</span>
        {/if}
    </div>

    {#if books.mirror_problem !== null}
        <div class="alert px-3 py-2 text-sm alert-warning" role="status" data-testid="books-mirror-problem">{books.mirror_problem}</div>
    {/if}

    {#if empty}
        <p class="text-sm text-base-content/70" data-testid="books-empty">
            {#if books.status === "draft"}
                Not booked: drafts never touch the books. Marking it sent books it.
            {:else if books.basis === "cash"}
                Nothing booked yet: cash books book an invoice when a payment is applied to it.
            {:else}
                Nothing in the books under this number.
            {/if}
        </p>
    {/if}

    {#if books.bookings.length > 0}
        <div class="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {#each books.bookings as booking, i (i)}
                {@render postings(booking, "Booked", "books-booking")}
            {/each}
        </div>
    {/if}

    {#if received.length > 0}
        <div data-testid="books-payments">
            <h3 class="text-sm font-medium">Payments and credits</h3>
            <table class="table mt-1 table-sm">
                <thead>
                    <tr>
                        <th>Date</th>
                        <th title="Where the money went, or the credit came from">Account</th>
                        <th>Description</th>
                        <th class="text-right">Amount</th>
                    </tr>
                </thead>
                <tbody>
                    {#each received as payment, i (i)}
                        <tr data-testid="books-payment">
                            <td class="whitespace-nowrap tabular-nums">{payment.date}</td>
                            <td class="font-mono text-xs break-all">{payment.from.join(", ")}</td>
                            <td>
                                {payment.description}
                                {#if payment.kind === "credit"}<span class="ml-1 badge badge-ghost badge-xs">credit</span>{/if}
                                <span class="block text-xs text-base-content/60">{payment.file}:{payment.line}</span>
                            </td>
                            <td class="text-right whitespace-nowrap tabular-nums">{payment.amount}</td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
    {/if}

    {#if books.payments.length > 0}
        <div class="grid grid-cols-1 gap-4 xl:grid-cols-2">
            {#each books.payments as payment, i (i)}
                {@render postings(payment, `Paid ${payment.amount}`, "books-cash-payment")}
            {/each}
        </div>
    {/if}

    {#each books.voids as reversal, i (i)}
        {@render postings(reversal, "Void", "books-void")}
    {/each}

    {#if books.problems.length > 0}
        <ul class="flex flex-col gap-1" data-testid="books-problems">
            {#each books.problems as problem, i (i)}
                <li class="alert px-3 py-2 text-xs {problem.severity === 'error' ? 'alert-error' : 'alert-warning'}">{problem.message}</li>
            {/each}
        </ul>
    {/if}
</section>
