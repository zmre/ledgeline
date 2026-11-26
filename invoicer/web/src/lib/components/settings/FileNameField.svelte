<!-- The new invoice file name (SETTINGS-PLAN §4): typed text and the Date,
     Customer and Number fields as chips, never the setting's own spelling,
     with a live example ("2026-11-25 Acme Invoice 1001.md"). Each piece of
     text is a small field; each chip has a remove button; Add Date / Add
     Customer / Add Number put one at the end. The extension stays as it is.
     Tab walks the pieces in reading order. -->
<script lang="ts">
    import {NAME_FIELDS, nameExample, nameFieldLabel, nameOf, nameParts, splitExtension, withField, withoutField, type NamePart} from "#lib/settingsForm.js";

    let {
        id,
        value,
        sample,
        disabled = false,
        onChange,
    }: {
        /** The first piece's id (the row's label points at it). */
        id: string;
        value: string;
        sample: {date: string; customer: string; number: string};
        disabled?: boolean;
        onChange: (value: string) => void;
    } = $props();

    const split = $derived(splitExtension(value));
    const parts = $derived(nameParts(split.stem));
    const example = $derived(nameExample(parts, split.extension, sample));

    function edit(next: NamePart[]): void {
        onChange(nameOf(next, split.extension));
    }

    function setText(at: number, text: string): void {
        edit(parts.map((p, i) => (i === at && p.kind === "text" ? {kind: "text", text} : p)));
    }

    /** A text piece's width: what it holds, and a little room to type. */
    const width = (text: string): string => `${Math.max(1, text.length) + 1}ch`;
</script>

<div class="flex flex-col gap-2" data-testid="filename-field">
    <div class="flex flex-wrap items-center gap-1 rounded-field border border-base-300 bg-base-100 p-1.5" role="group" aria-label="File name pieces">
        {#each parts as part, at (at)}
            {#if part.kind === "text"}
                <input
                    id={at === 0 ? id : undefined}
                    type="text"
                    class="input min-w-[2ch] border-transparent px-1 shadow-none input-xs"
                    style:width={width(part.text)}
                    value={part.text}
                    aria-label={at === 0 ? "Text at the start" : `Text after ${nameFieldLabel((parts[at - 1] as {field: string}).field)}`}
                    {disabled}
                    oninput={(e) => setText(at, e.currentTarget.value)}
                    data-testid="filename-text-{at}"
                />
            {:else}
                <span class="badge gap-1 badge-primary" data-testid="filename-chip-{part.field}">
                    {nameFieldLabel(part.field)}
                    <button
                        type="button"
                        class="btn btn-circle btn-ghost btn-xs"
                        aria-label="Remove {nameFieldLabel(part.field)}"
                        {disabled}
                        onclick={() => edit(withoutField(parts, at))}
                        data-testid="filename-remove-{part.field}">×</button
                    >
                </span>
            {/if}
        {/each}
        {#if split.extension !== ""}<span class="px-1 text-sm text-base-content/60">{split.extension}</span>{/if}
    </div>
    <div class="flex flex-wrap items-center gap-1.5">
        {#each NAME_FIELDS as f (f.field)}
            <button type="button" class="btn btn-outline btn-xs" {disabled} onclick={() => edit(withField(parts, f.field))} data-testid="filename-add-{f.field}"
                >Add {f.label}</button
            >
        {/each}
    </div>
    <p class="text-sm text-base-content/70">For example: <span class="font-medium break-all" data-testid="filename-example">{example}</span></p>
</div>
