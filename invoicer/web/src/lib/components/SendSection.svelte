<!-- The Send panel with its wiring (PLAN 3c): the delivery state, Export PDF
     and its prompts, Share…, Copy file, Open, the portal. One component, so
     the invoice page and the editor show the same panel rather than two
     copies of its handlers. `reload` bumps re-fetch the delivery (a change on
     disk); `onChanged` tells the page the invoice was sent or exported. -->
<script lang="ts">
    import {copyPdf, exportPdf, fetchDelivery, openPdf, openPortal, sendInvoice, sharePdf} from "#lib/api.js";
    import type {Anchor, Delivery, Destination, Exported} from "#lib/delivery.js";
    import SendPanel from "./SendPanel.svelte";

    let {number, reload = 0, onChanged}: {number: string; reload?: number; onChanged?: () => void} = $props();

    let delivery = $state<Delivery | null>(null);
    let exported = $state<Exported | null>(null);
    let busy = $state(false);
    let error = $state<string | null>(null);

    const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

    function load(target: string): void {
        fetchDelivery(target)
            .then((d) => {
                if (target === number) delivery = d;
            })
            .catch((e: unknown) => (error = message(e)));
    }

    async function act(action: () => Promise<void>): Promise<void> {
        busy = true;
        error = null;
        try {
            await action();
        } catch (e: unknown) {
            error = message(e);
        } finally {
            busy = false;
        }
    }

    $effect(() => {
        const target = number;
        void reload;
        load(target);
    });

    $effect(() => {
        void number;
        exported = null;
        error = null;
    });

    const onExport = (options: {temp: boolean}): Promise<void> =>
        act(async () => {
            const target = number;
            exported = await exportPdf(target, options);
            load(target);
            onChanged?.();
        });

    const onSendAndExport = (via: string): Promise<void> =>
        act(async () => {
            const target = number;
            await sendInvoice(target, via);
            onChanged?.();
            exported = await exportPdf(target);
            load(target);
            onChanged?.();
        });

    const onShare = (destination: Destination, file: string, anchor: Anchor): Promise<void> => act(() => sharePdf(destination, file, anchor));
    const onCopyFile = (destination: Destination, file: string): Promise<void> => act(() => copyPdf(destination, file));
    const onOpenFile = (destination: Destination, file: string): Promise<void> => act(() => openPdf(destination, file));
    const onOpenPortal = (): Promise<void> => act(() => openPortal(number));
    const onCopyText = (label: string, text: string): Promise<void> =>
        act(async () => {
            try {
                await navigator.clipboard.writeText(text);
            } catch {
                throw new Error(`Couldn't copy the ${label.toLowerCase()}; select it and copy by hand.`);
            }
        });
</script>

{#if delivery !== null}
    <SendPanel {delivery} {exported} {busy} {error} {onExport} {onSendAndExport} {onShare} {onCopyFile} {onOpenFile} {onOpenPortal} {onCopyText} />
{:else if error !== null}
    <div class="alert text-sm alert-error" role="alert" data-testid="send-error">{error}</div>
{/if}
