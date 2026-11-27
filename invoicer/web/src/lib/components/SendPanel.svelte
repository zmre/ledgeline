<!-- Sending an invoice (invoicer PLAN 3c, ARCHITECTURE §5.3, §3.4): its PDF,
     the "Mark as sent?" prompt a draft's export asks, the warning when a sent
     invoice changed since its PDF was kept, the PDFs kept so far, Share… and
     Copy file, and for a customer with a vendor portal its instructions, Open
     portal and copy buttons. Presentational: the page does the calls, so every
     action is a callback, and `busy` and `error` come back in.

     The current PDF (this session's export, else the latest kept) is one line,
     and with a window it opens in the default PDF viewer on a click, as does
     each earlier one (2026-11-23). Laid out across the page, above the
     invoice: actions and prompts on the left, the portal and the PDFs on the
     right.

     Keyboard (PLAN 5k): Shift+P exports the PDF, as the button does; a
     prompt it raises takes focus, on its first choice. -->
<script lang="ts">
    import {tick} from "svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {
        anchorOf,
        currentPdf,
        defaultVia,
        exportPrompt,
        sendBlocker,
        sizeText,
        VIA_CHOICES,
        type Anchor,
        type Delivery,
        type Destination,
        type Exported,
    } from "#lib/delivery.js";

    interface Props {
        delivery: Delivery;
        /** The last export this session, to share or copy. */
        exported?: Exported | null;
        busy?: boolean;
        error?: string | null;
        onExport: (options: {temp: boolean}) => void;
        /** Mark it sent (`via`), then export the kept PDF. */
        onSendAndExport: (via: string) => void;
        onShare: (destination: Destination, file: string, anchor: Anchor) => void;
        onCopyFile: (destination: Destination, file: string) => void;
        /** Open an exported PDF in the default viewer. */
        onOpenFile: (destination: Destination, file: string) => void;
        onOpenPortal: () => void;
        onCopyText: (label: string, text: string) => void;
    }

    let {
        delivery,
        exported = null,
        busy = false,
        error = null,
        onExport,
        onSendAndExport,
        onShare,
        onCopyFile,
        onOpenFile,
        onOpenPortal,
        onCopyText,
    }: Props = $props();

    /** The prompt on screen, if any. */
    let asking = $state<"mark-sent" | "changed" | null>(null);
    let via = $state("");

    const caps = $derived(delivery.capabilities);
    const blocker = $derived(sendBlocker(delivery));
    const portal = $derived(delivery.portal);
    const viaChoice = $derived(via === "" ? defaultVia(delivery) : via);
    /** The current PDF, one line: what Share…, Copy file and the click act on. */
    const current = $derived(currentPdf(delivery, exported));
    const target = $derived(current?.handle ?? null);
    /** The kept PDFs not already on the current line. */
    const earlier = $derived(delivery.exports.filter((pdf) => !(target?.destination === "kept" && target.file === pdf.file)));

    function clickExport(): void {
        const prompt = exportPrompt(delivery);
        if (prompt === "none") onExport({temp: false});
        else asking = prompt;
    }

    let sectionEl = $state<HTMLElement | null>(null);

    /** Export from the keyboard: as the button, then into any prompt it raised. */
    async function exportFromKeys(): Promise<void> {
        clickExport();
        await tick();
        sectionEl?.querySelector<HTMLElement>('[data-testid="send-prompt"] button')?.focus();
    }

    registerKeys({
        id: "send-panel",
        bindings: [
            {
                keys: "P",
                label: "Export the PDF",
                group: "Invoice",
                enabled: () => caps.pdf && !busy && delivery.status !== null && asking === null,
                run: () => void exportFromKeys(),
            },
        ],
    });

    function share(event: MouseEvent): void {
        if (target === null) return;
        const button = event.currentTarget as HTMLElement;
        onShare(target.destination, target.file, anchorOf(button.getBoundingClientRect()));
    }

    const copyFields = $derived(
        [
            {label: "Number", value: delivery.copy.number},
            {label: "Reference", value: delivery.copy.reference},
            {label: "Total", value: delivery.copy.total},
            {label: "Supplier ID", value: portal?.supplier_id ?? null},
            {label: "Login", value: portal?.login ?? null},
        ].filter((f): f is {label: string; value: string} => f.value !== null && f.value !== "")
    );
</script>

<section
    class="grid grid-cols-1 gap-4 rounded-box border border-base-300 p-4 md:grid-cols-2"
    aria-labelledby="send-heading"
    bind:this={sectionEl}
    data-testid="send-panel"
>
    <div class="flex min-w-0 flex-col gap-3">
        <div class="flex items-center justify-between">
            <h2 id="send-heading" class="text-sm font-semibold tracking-wide uppercase">Send</h2>
            {#if delivery.status === "sent" && delivery.sent_date}
                <span class="text-xs text-base-content/70" data-testid="send-sent">
                    Sent {delivery.sent_date}{delivery.sent_via ? ` by ${delivery.sent_via}` : ""}
                </span>
            {/if}
        </div>

        {#if delivery.changed_since_sent === true}
            <div class="alert text-sm alert-warning" role="status" data-testid="send-changed">
                Changed since its PDF was kept: a new PDF will differ from what the customer has.
            </div>
        {/if}

        {#if error !== null}
            <div class="alert text-sm alert-error" role="alert" data-testid="send-error">{error}</div>
        {/if}

        {#if caps.pdf}
            <div class="flex flex-wrap gap-2">
                <button
                    type="button"
                    class="btn btn-primary btn-sm"
                    disabled={busy || delivery.status === null}
                    onclick={clickExport}
                    data-testid="send-export"
                >
                    {#if busy}<span class="loading loading-xs loading-spinner"></span>{/if}
                    {delivery.status === "sent" ? "Export PDF" : "Export PDF…"}
                </button>
                {#if caps.share && target !== null}
                    <button type="button" class="btn btn-outline btn-sm" disabled={busy} onclick={share} data-testid="send-share">Share…</button>
                {/if}
                {#if caps.copy_file && target !== null}
                    <button
                        type="button"
                        class="btn btn-outline btn-sm"
                        disabled={busy}
                        onclick={() => target && onCopyFile(target.destination, target.file)}
                        data-testid="send-copy-file"
                    >
                        Copy file
                    </button>
                {/if}
            </div>
        {:else}
            <p class="text-sm text-base-content/70" data-testid="send-no-pdf">PDF export needs the desktop app.</p>
        {/if}

        {#if asking === "mark-sent"}
            <div class="rounded-box border border-base-300 bg-base-200 p-3" role="alertdialog" aria-labelledby="mark-sent-title" data-testid="send-prompt">
                <p id="mark-sent-title" class="font-medium">Mark invoice {delivery.number} as sent?</p>
                <p class="mt-1 text-sm text-base-content/70">Sending books it, and its PDF is kept in {delivery.pdf_dir}. A draft PDF is only a look.</p>
                {#if blocker !== null}
                    <p class="mt-2 text-sm text-error-text" data-testid="send-blocker">{blocker}</p>
                {/if}
                <label class="mt-2 flex items-center gap-2 text-sm">
                    <span class="text-base-content/70">Sent by</span>
                    <select class="select-bordered select select-xs" value={viaChoice} onchange={(e) => (via = e.currentTarget.value)} data-testid="send-via">
                        {#each VIA_CHOICES as choice (choice)}
                            <option value={choice}>{choice}</option>
                        {/each}
                    </select>
                </label>
                <div class="mt-3 flex flex-wrap gap-2">
                    <button
                        type="button"
                        class="btn btn-primary btn-sm"
                        disabled={blocker !== null}
                        onclick={() => {
                            asking = null;
                            onSendAndExport(viaChoice);
                        }}
                        data-testid="send-mark-and-export">Mark as sent and export</button
                    >
                    <button
                        type="button"
                        class="btn btn-sm"
                        onclick={() => {
                            asking = null;
                            onExport({temp: true});
                        }}
                        data-testid="send-draft-only">Export a draft PDF</button
                    >
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (asking = null)}>Cancel</button>
                </div>
            </div>
        {:else if asking === "changed"}
            <div class="rounded-box border border-warning bg-base-200 p-3" role="alertdialog" aria-labelledby="changed-title" data-testid="send-prompt">
                <p id="changed-title" class="font-medium">Invoice {delivery.number} changed since it was sent</p>
                <p class="mt-1 text-sm text-base-content/70">The new PDF will differ from what the customer has. The earlier PDF is kept under its date.</p>
                <div class="mt-3 flex gap-2">
                    <button
                        type="button"
                        class="btn btn-sm btn-warning"
                        onclick={() => {
                            asking = null;
                            onExport({temp: false});
                        }}
                        data-testid="send-export-anyway">Export anyway</button
                    >
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (asking = null)}>Cancel</button>
                </div>
            </div>
        {/if}

        {#if current !== null}
            <div class="text-sm" role={exported !== null ? "status" : undefined} data-testid={exported !== null ? "send-exported" : "send-current"}>
                {#if caps.open_file}
                    <button
                        type="button"
                        class="link text-left break-all link-hover"
                        title="Open in the default PDF viewer"
                        onclick={() => current && onOpenFile(current.handle.destination, current.handle.file)}
                        data-testid="send-open-pdf">{current.text}</button
                    >
                {:else}
                    <span class="break-all">{current.text}</span>
                {/if}
                {#if exported?.earlier}<span class="block text-xs text-base-content/70">The earlier one is kept as {exported.earlier}.</span>{/if}
                {#each exported?.warnings ?? [] as warning, i (i)}
                    <span class="block text-xs text-warning">{warning}</span>
                {/each}
            </div>
        {/if}
    </div>

    <div class="flex min-w-0 flex-col gap-3">
        {#if portal !== null}
            <div class="rounded-box bg-base-200 p-3" data-testid="send-portal">
                <div class="flex items-center justify-between gap-2">
                    <span class="text-sm font-medium">{portal.name ?? "Vendor portal"}</span>
                    {#if portal.url !== null}
                        <button type="button" class="btn btn-outline btn-xs" onclick={onOpenPortal} data-testid="send-open-portal">Open portal</button>
                    {/if}
                </div>
                <p class="mt-1 text-xs text-base-content/70">Export the PDF (sent by portal), upload it there, then share or copy it as the portal asks.</p>
                {#if portal.instructions}
                    <p class="mt-2 text-sm whitespace-pre-line" data-testid="send-instructions">{portal.instructions}</p>
                {/if}
                {#if copyFields.length > 0}
                    <dl class="mt-2 grid grid-cols-[auto_1fr_auto] items-center gap-x-2 gap-y-1 text-sm">
                        {#each copyFields as field (field.label)}
                            <dt class="text-base-content/70">{field.label}</dt>
                            <dd class="truncate tabular-nums">{field.value}</dd>
                            <dd>
                                <button
                                    type="button"
                                    class="btn btn-ghost btn-xs"
                                    aria-label="Copy {field.label.toLowerCase()}"
                                    onclick={() => onCopyText(field.label, field.value)}
                                    data-testid="send-copy-{field.label.toLowerCase().replace(' ', '-')}">Copy</button
                                >
                            </dd>
                        {/each}
                    </dl>
                {/if}
            </div>
        {/if}

        {#if earlier.length > 0}
            <div data-testid="send-kept">
                <div class="text-xs text-base-content/70">Earlier PDFs in {delivery.pdf_dir}</div>
                <ul class="text-sm">
                    {#each earlier as pdf (pdf.file)}
                        <li class="flex justify-between gap-2" data-testid="send-kept-pdf">
                            {#if caps.open_file}
                                <button
                                    type="button"
                                    class="link truncate text-left link-hover {pdf.latest ? 'font-medium' : 'text-base-content/70'}"
                                    title="Open in the default PDF viewer"
                                    onclick={() => onOpenFile("kept", pdf.file)}>{pdf.file}</button
                                >
                            {:else}
                                <span class="truncate {pdf.latest ? 'font-medium' : 'text-base-content/70'}">{pdf.file}</span>
                            {/if}
                            <span class="shrink-0 text-xs text-base-content/60 tabular-nums">{pdf.date ?? ""} · {sizeText(pdf.bytes)}</span>
                        </li>
                    {/each}
                </ul>
            </div>
        {/if}
    </div>
</section>
