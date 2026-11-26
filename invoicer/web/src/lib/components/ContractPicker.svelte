<!-- Attach a note or file (Phase 4 feedback J5): a contract, or a PO's
     attachment. A dialog to find a note or file in the books folder by
     name; in the window it also offers the system's open dialog ("Choose a
     file…"), which takes a file anywhere (one outside the books folder is
     recorded as `~/…` or a `file:///` URL). The chosen entry is what the note
     writes (`[[Title]]` for a note, a `/`-rooted path for a file); the user
     only ever sees names. A daisyUI modal drawn open (a div, so it mounts in
     jsdom); Escape closes it. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {fetchFiles, kindLabel, pickFile, type FileCandidate, type FileList} from "#lib/customers.js";
    import {errorText} from "#lib/labels.js";

    let {
        onPick,
        onClose,
        exclude = [],
        title = "Add a contract",
        load = fetchFiles,
        pick = pickFile,
    }: {
        onPick: (file: FileCandidate) => void;
        onClose: () => void;
        /** Entries already in the list, not offered again. */
        exclude?: string[];
        /** The dialog's heading ("Add a contract", "Attach a file to PO #105"). */
        title?: string;
        load?: () => Promise<FileList>;
        pick?: () => Promise<FileCandidate | null>;
    } = $props();

    /** The most candidates listed at once: type to narrow. */
    const SHOWN = 50;

    const uid = $props.id();
    let list = $state<FileList | null>(null);
    let failure = $state<string | null>(null);
    let query = $state("");
    let picking = $state(false);

    const matches = $derived.by((): FileCandidate[] => {
        if (list === null) return [];
        const words = query
            .toLowerCase()
            .split(/\s+/)
            .filter((w) => w !== "");
        const taken = new Set(exclude);
        return list.files.filter((f) => !taken.has(f.entry) && words.every((w) => `${f.name} ${f.path}`.toLowerCase().includes(w)));
    });

    onMount(() => {
        load()
            .then((l) => (list = l))
            .catch((e: unknown) => (failure = errorText(e)));
    });

    async function choose(): Promise<void> {
        picking = true;
        failure = null;
        try {
            const file = await pick();
            if (file !== null) onPick(file);
        } catch (e: unknown) {
            failure = errorText(e);
        } finally {
            picking = false;
        }
    }

    function folderOf(path: string): string {
        const at = path.lastIndexOf("/");
        return at === -1 ? "" : path.slice(0, at);
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onClose();
        }
    }

    function focus(node: HTMLInputElement): void {
        node.focus();
    }
</script>

<div class="modal modal-open" role="presentation" onkeydown={onKeydown}>
    <div class="modal-box flex max-h-[85vh] flex-col gap-3" role="dialog" aria-modal="true" aria-labelledby="{uid}-title" data-testid="contract-picker">
        <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
        <p class="text-sm text-base-content/70">
            {list?.pick ? "A note or file in the books folder, or choose any file." : "A note or file (a signed PDF, say) in the books folder."}
        </p>
        {#if list?.pick}
            <div>
                <button type="button" class="btn btn-sm" disabled={picking} onclick={() => void choose()} data-testid="contract-choose-file">
                    {picking ? "Choosing…" : "Choose a file…"}
                </button>
            </div>
        {/if}
        <label class="input w-full input-sm">
            <span class="sr-only">Find a note or file</span>
            <input type="search" placeholder="Find by name" bind:value={query} use:focus data-testid="contract-search" />
        </label>
        {#if failure !== null}
            <div role="alert" class="alert alert-soft text-sm alert-error" data-testid="contract-picker-error">{failure}</div>
        {/if}
        {#if list === null && failure === null}
            <span class="loading loading-sm loading-spinner" aria-label="Loading the books' files"></span>
        {:else if list !== null}
            {#if matches.length === 0}
                <p class="text-sm text-base-content/70" data-testid="contract-no-match">Nothing in the books folder matches.</p>
            {:else}
                <ul class="min-h-0 overflow-y-auto rounded-box border border-base-300" data-testid="contract-candidates">
                    {#each matches.slice(0, SHOWN) as file (file.path)}
                        {@const kind = kindLabel(file.kind)}
                        <li>
                            <button
                                type="button"
                                class="flex w-full flex-wrap items-baseline gap-x-2 px-3 py-1.5 text-left text-sm hover:bg-base-200 focus:bg-base-200"
                                onclick={() => onPick(file)}
                                data-testid="contract-candidate"
                            >
                                <span class="font-medium">{file.name}</span>
                                {#if kind !== null}<span class="badge badge-ghost badge-xs">{kind}</span>{/if}
                                <span class="ml-auto truncate text-xs text-base-content/60">{folderOf(file.path)}</span>
                            </button>
                        </li>
                    {/each}
                </ul>
                {#if matches.length > SHOWN}
                    <p class="text-xs text-base-content/60">{matches.length - SHOWN} more: type to narrow the list.</p>
                {/if}
            {/if}
        {/if}
        <div class="modal-action mt-0">
            <button type="button" class="btn btn-ghost btn-sm" onclick={onClose}>Cancel</button>
        </div>
    </div>
</div>
