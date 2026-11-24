<!-- The templates list (invoicer PLAN 4d): each template by name, what it is
     for, what it builds on (the invoice base and its version, or a whole
     page), the blocks it changes, and who uses it. A starter not copied into
     the books is marked: it opens read-only, with Customize. Names are links
     to the editor; links arrive resolved. The table scrolls in its own box
     in a narrow window (J12). -->
<script lang="ts">
    import {baseText, kindsText, usageText, type TemplateListEntry} from "#lib/templates.js";

    let {entries, href}: {entries: TemplateListEntry[]; href: (entry: TemplateListEntry) => string} = $props();

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
                <tr data-testid="template-row" data-path={entry.path}>
                    <td class="min-w-40">
                        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page resolves every link -->
                        <a class="link font-medium link-hover" href={href(entry)} data-testid="template-link">{entry.name}</a>
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
