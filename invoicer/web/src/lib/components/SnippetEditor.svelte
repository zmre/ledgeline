<!-- One snippet's form (invoicer PLAN 4c, ARCHITECTURE §3.7): its title (what
     a customer's `snippets:` links), its kind, its markdown body in MBR's
     field (a textarea when that can't load), and where it is used: the
     customers whose new drafts copy it, and the invoices holding a copy of
     its body as it is now. Changing a snippet never changes an invoice. -->
<script lang="ts">
    import Combobox from "#ui/components/Combobox.svelte";
    import type {ComboOption} from "#ui/ui/comboMatch.js";
    import {SNIPPET_KINDS, type SnippetEntry, type SnippetForm, type SnippetUse} from "#lib/library.js";
    import ProseField from "./ProseField.svelte";

    let {
        form = $bindable(),
        entry,
        customerHref,
        invoiceHref,
    }: {
        form: SnippetForm;
        /** The snippet as saved (its uses, aliases and path). */
        entry: SnippetEntry;
        customerHref: (use: SnippetUse) => string;
        invoiceHref: (use: SnippetUse) => string;
    } = $props();

    const KINDS: ComboOption[] = SNIPPET_KINDS.map((kind) => ({value: kind, label: kind}));
    const customers = $derived(entry.used_by.filter((u) => u.how === "default"));
    const copies = $derived(entry.used_by.filter((u) => u.how === "copy"));
    const folder = $derived(`/${entry.path.split("/").slice(0, -1).join("/")}/`);
</script>

<div class="flex flex-col gap-4" data-testid="snippet-editor">
    <div class="grid gap-3 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
        <label class="flex flex-col gap-1 text-sm">
            <span class="font-medium">Title</span>
            <input class="input w-full input-sm" bind:value={form.title} data-testid="snippet-title" />
            <span class="text-xs text-base-content/60">Customers link it as <code>[[{form.title.trim() || "Title"}]]</code>.</span>
        </label>
        <div class="flex flex-col gap-1 text-sm">
            <span class="font-medium" aria-hidden="true">Kind</span>
            <Combobox bind:value={form.kind} options={KINDS} label="Kind" placeholder="payment, terms, note" size="sm" testid="snippet-kind" />
        </div>
    </div>
    {#if entry.aliases.length > 0}
        <p class="text-xs text-base-content/60" data-testid="snippet-aliases">Also answers to {entry.aliases.map((a) => `[[${a}]]`).join(", ")}.</p>
    {/if}

    <div class="flex flex-col gap-1 text-sm">
        <span class="font-medium" aria-hidden="true">Text</span>
        <ProseField value={form.body} label="Snippet text" linkBase={folder} onChange={(value) => (form.body = value)} testid="snippet-body" />
        <span class="text-xs text-base-content/60">Markdown. An invoice gets a copy when the snippet is inserted; it doesn't follow later edits.</span>
    </div>

    <section class="rounded-box border border-base-300 p-4 text-sm" data-testid="snippet-uses">
        <h2 class="mb-2 text-sm font-semibold tracking-wide text-base-content/70 uppercase">Used by</h2>
        {#if entry.used_by.length === 0}
            <p class="text-base-content/60">No customer names it and no invoice holds its text.</p>
        {/if}
        {#if customers.length > 0}
            <p class="text-xs text-base-content/60">A default for these customers: their new invoices copy it.</p>
            <ul class="mb-2" data-testid="snippet-customers">
                <!-- eslint-disable svelte/no-navigation-without-resolve -- the page resolves every link -->
                {#each customers as use (use.path)}<li><a class="link link-hover" href={customerHref(use)}>{use.title}</a></li>{/each}
                <!-- eslint-enable svelte/no-navigation-without-resolve -->
            </ul>
        {/if}
        {#if copies.length > 0}
            <p class="text-xs text-base-content/60">Invoices holding its text as it is now:</p>
            <ul data-testid="snippet-copies">
                <!-- eslint-disable svelte/no-navigation-without-resolve -- the page resolves every link -->
                {#each copies as use (use.path)}<li><a class="link link-hover" href={invoiceHref(use)}>{use.number ?? use.title}</a></li>{/each}
                <!-- eslint-enable svelte/no-navigation-without-resolve -->
            </ul>
        {/if}
    </section>
</div>
