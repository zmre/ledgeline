<!-- The templates list (invoicer PLAN 4d): each template by name, what it is
     for, what it builds on (the invoice base and its version, or a whole
     page), the blocks it changes, and who uses it. A starter not copied into
     the books is marked: it opens read-only, with Customize. Links arrive
     resolved. The table scrolls in its own box in a narrow window (J12).

     A click anywhere on a row opens the template, as in the invoice and
     customer lists (`lib/rows.ts`): the name is the row's one link, for the
     keyboard and screen readers. -->
<script lang="ts">
    import {rowClick} from "#lib/rows.js";
    import {baseText, kindsText, usageText, type TemplateListEntry} from "#lib/templates.js";

    let {entries, href, onOpen}: {entries: TemplateListEntry[]; href: (entry: TemplateListEntry) => string; onOpen?: (entry: TemplateListEntry) => void} =
        $props();

    /** Who uses it, by name, for the tooltip. */
    const usedByTitle = (entry: TemplateListEntry): string | undefined =>
        entry.used_by.length === 0
            ? undefined
            : entry.used_by.map((u) => (u.how === "invoice" ? `Invoice ${u.number ?? u.title}` : `${u.title} (new invoices)`)).join(", ");
</script>

<div class="overflow-x-auto rounded-box border border-base-300">
    <table class="table table-sm" data-testid="template-list">
        <thead>
            <tr>
                <th>Name</th>
                <th>For</th>
                <th>Builds on</th>
                <th>Changes</th>
                <th>Used by</th>
            </tr>
        </thead>
        <tbody>
            {#each entries as entry (entry.path)}
                <tr
                    class="cursor-pointer hover:bg-base-200"
                    data-testid="template-row"
                    data-path={entry.path}
                    onclick={(event) => rowClick(event, onOpen === undefined ? undefined : () => onOpen(entry))}
                >
                    <td class="min-w-40">
                        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page resolves every link -->
                        <a class="link font-medium link-hover" href={href(entry)} data-testid="template-link" data-row-link>{entry.name}</a>
                        {#if !entry.installed}
                            <span
                                class="ml-1 badge badge-ghost badge-sm"
                                title="Shipped with the app; Customize copies it into your books"
                                data-testid="template-starter">Starter</span
                            >
                        {/if}
                    </td>
                    <td class="whitespace-nowrap">{kindsText(entry.kinds)}</td>
                    <td class="whitespace-nowrap">{baseText(entry)}</td>
                    <td>{entry.extends === null ? "—" : entry.overrides.length === 0 ? "Nothing yet" : entry.overrides.join(", ")}</td>
                    <td class="whitespace-nowrap" title={usedByTitle(entry)} data-testid="template-usage">{usageText(entry)}</td>
                </tr>
            {/each}
        </tbody>
    </table>
</div>
