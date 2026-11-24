<!-- The editor's Books panel (ARCHITECTURE §6.3): for a draft, the
     transaction sending would write (drafts never touch the books, I2); for a
     sent invoice, what is booked and whether it is still what the file says,
     with Re-book when it isn't. The saved invoice's full Books panel
     (payments, schedules) is on its own page. -->
<script lang="ts">
    import type {WouldBook} from "#lib/editor.js";

    let {
        books,
        status,
        dirty = false,
        onRebook,
    }: {
        books: WouldBook | null;
        status: string;
        dirty?: boolean;
        onRebook?: () => void;
    } = $props();
</script>

<section class="flex flex-col gap-2 rounded-box border border-base-300 p-3" aria-label="Books" data-testid="would-book">
    <h2 class="font-semibold">Books</h2>
    {#if books === null}
        <span class="loading loading-sm loading-spinner" aria-label="Loading the books"></span>
    {:else if books.error !== null}
        <p class="text-sm text-error" data-testid="would-book-error">{books.error}</p>
    {:else}
        {#if status === "draft"}
            {#if books.would_book !== null}
                <p class="text-sm text-base-content/70">Sending it books{dirty ? " (with your unsaved edits)" : ""}:</p>
                <pre class="overflow-x-auto rounded-box bg-base-200 p-2 text-xs" data-testid="would-book-text">{books.would_book}</pre>
            {:else if books.basis === "cash"}
                <p class="text-sm text-base-content/70">Cash books: sending writes only the markdown; the books take the invoice when a payment is applied.</p>
            {:else}
                <p class="text-sm text-base-content/70">Nothing to book.</p>
            {/if}
        {:else}
            {#if books.booked !== null}
                <p class="text-sm text-base-content/70">Booked:</p>
                <pre class="overflow-x-auto rounded-box bg-base-200 p-2 text-xs" data-testid="booked-text">{books.booked.text}</pre>
            {:else}
                <p class="text-sm text-base-content/70">Nothing is booked under this number.</p>
            {/if}
            {#if books.in_step === false && books.would_book !== null}
                <div class="alert text-sm alert-warning" role="status" data-testid="books-out-of-step">
                    <div class="flex flex-col gap-1">
                        <span>The books are out of date with {dirty ? "these edits" : "the file"}. Re-booking writes:</span>
                        <pre class="overflow-x-auto text-xs">{books.would_book}</pre>
                    </div>
                    {#if onRebook !== undefined && !dirty && status === "sent"}
                        <button type="button" class="btn btn-sm" onclick={onRebook} data-testid="books-rebook">Re-book…</button>
                    {/if}
                </div>
            {/if}
        {/if}
    {/if}
</section>
