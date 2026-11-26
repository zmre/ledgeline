<!-- A contract or PO attachment, read-only (Phase 4 feedback J6, J7): by
     name, never `[[…]]` or a path. A note opens in the app's note view; a
     file in its viewer (the window) or a new browser tab (MBR serves the
     books folder). A file outside the books folder carries a small badge
     and where it is; it opens in its viewer in the window, and in a browser
     it has no link (nothing serves it), so its name and place show as text.
     A missing one says so. -->
<script lang="ts">
    import type {Attachment} from "#lib/customers.js";

    let {
        attachment,
        noteHref,
        fileHref,
        onOpenFile,
        testid = "attachment",
    }: {
        attachment: Attachment;
        /** The app's view of a note (by its path). */
        noteHref: (path: string) => string;
        /** Where a file in the books opens in a browser tab. */
        fileHref: (path: string) => string;
        /** Open a file in its viewer (the window): a books path, or an outside file's entry. Absent: a link, or text outside the books. */
        onOpenFile?: (path: string) => void;
        testid?: string;
    } = $props();

    const outside = $derived(attachment.kind === "outside" ? attachment : null);
</script>

<span class="inline-flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5" data-testid={testid} data-kind={attachment.kind}>
    {#if attachment.found === null}
        <span class="break-words">{attachment.name}</span>
        <span class="badge badge-sm badge-warning" title="The file or note can't be found">missing</span>
    {:else if attachment.kind === "note"}
        {@const found = attachment.found}
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- the page passes an href built with resolve() -->
        <a class="link break-words" href={noteHref(found)}>{attachment.name}</a>
    {:else if outside !== null}
        {#if onOpenFile !== undefined}
            {@const entry = outside.path}
            <button type="button" class="link text-left break-words" onclick={() => onOpenFile(entry)}>{attachment.name}</button>
        {:else}
            <span class="break-words">{attachment.name}</span>
        {/if}
    {:else if onOpenFile !== undefined}
        {@const found = attachment.found}
        <button type="button" class="link text-left break-words" onclick={() => onOpenFile(found)}>{attachment.name}</button>
    {:else}
        <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- a file MBR serves, in its own tab -->
        <a class="link break-words" href={fileHref(attachment.found)} target="_blank" rel="noopener">{attachment.name}</a>
    {/if}
    {#if outside !== null}
        <span class="badge badge-ghost badge-sm" data-testid="{testid}-outside">outside the books</span>
        <span class="text-xs break-all text-base-content/60">{outside.location}</span>
    {/if}
</span>
