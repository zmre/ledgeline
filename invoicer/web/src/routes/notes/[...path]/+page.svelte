<!-- Any other note, inside the app (Phase 4 feedback J9): MBR's page for it
     in a frame under the app's sidebar, with a way back. Customers and
     invoices have their own pages; a contract, a meeting note or a person
     who isn't a customer opens here, never as MBR's page in place of the
     app. The frame is sandboxed with no scripts (NoteFrame), so its links
     move only the frame. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {ensureSession} from "#lib/api.js";
    import {pageUrl} from "#lib/invoices.js";
    import NoteFrame from "#lib/components/NoteFrame.svelte";

    const path = $derived(page.params.path ?? "");
    const name = $derived(path.split("/").pop() || path);
    let sessionReady = $state(false);
    let sessionError = $state<string | null>(null);

    onMount(() => {
        ensureSession()
            .then(() => (sessionReady = true))
            .catch((e: unknown) => (sessionError = e instanceof Error ? e.message : String(e)));
    });

    function back(event: MouseEvent): void {
        if (history.length > 1) {
            event.preventDefault();
            history.back();
        }
    }
</script>

<svelte:head>
    <title>{name} · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex h-full max-w-7xl flex-col gap-3" data-testid="note-page">
    <a class="link text-sm link-hover" href={resolve("/customers")} onclick={back} data-testid="note-back">← Back</a>
    <h1 class="text-2xl font-semibold tracking-tight break-words" data-testid="note-heading">{name}</h1>
    {#if sessionError !== null}
        <div role="alert" class="alert alert-soft alert-error">{sessionError}</div>
    {:else if sessionReady}
        <NoteFrame src={pageUrl(path)} title={name} height={0} />
    {/if}
</div>
