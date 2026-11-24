<!-- The invoice's template (invoicer PLAN, Phase 3 feedback F9): which one it
     uses, a picker that previews another, and "Use this template", which
     writes the invoice's `template:` (a starter is copied into the books
     then). Presentational: the page does the call, so `onUse` is a callback
     and `busy` and `error` come back in. -->
<script lang="ts">
    import {templateChoices, templateName, type TemplateEntry} from "#lib/invoices.js";

    interface Props {
        templates: TemplateEntry[];
        /** The `template:` the invoice names (null: none). */
        own: string | null;
        /** The template the frame shows: "" for the invoice's own. */
        chosen?: string;
        busy?: boolean;
        error?: string | null;
        /** Make `template` the invoice's own. */
        onUse: (template: string) => void;
    }

    let {templates, own, chosen = $bindable(""), busy = false, error = null, onUse}: Props = $props();

    const choices = $derived(templateChoices(templates, own));
    const current = $derived(templateName(templates, own));
    const showingOwn = $derived(chosen === "" || chosen === own);
    const picked = $derived(showingOwn ? null : (templates.find((t) => t.path === chosen) ?? null));
</script>

<div class="flex flex-col gap-1" data-testid="template-picker">
    <div class="flex flex-wrap items-center gap-2">
        <label class="text-sm text-base-content/70" for="template-choice">Template</label>
        <select id="template-choice" class="select-bordered select select-sm" bind:value={chosen} disabled={busy} data-testid="template-choice">
            {#if own === null || current?.known === false}
                <option value="">{own === null ? "As MBR shows the note" : `${own} (this invoice's, not found)`}</option>
            {/if}
            {#each choices as template (template.path)}
                <option value={template.path === own ? "" : template.path}>
                    {template.name}{template.path === own ? " (this invoice's)" : template.installed ? "" : " (starter)"}
                </option>
            {/each}
        </select>
        {#if picked !== null}
            <button type="button" class="btn btn-primary btn-sm" disabled={busy} onclick={() => onUse(picked.path)} data-testid="template-use">
                {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                Use this template
            </button>
        {/if}
    </div>
    <p class="text-xs text-base-content/60" data-testid="template-current">
        {#if picked !== null}
            Previewing {picked.name}: the invoice still uses {current?.name ?? "no template"}.{picked.installed
                ? ""
                : " Using it copies the starter into the books, where it is yours to edit."}
        {:else if current === null}
            This invoice names no template.
        {:else if current.known}
            This invoice uses {current.name} ({own}).
        {:else}
            This invoice names {own}, which isn't in the books: pick a template and use it.
        {/if}
    </p>
    {#if error !== null}
        <div class="alert text-sm alert-error" role="alert" data-testid="template-error">{error}</div>
    {/if}
</div>
