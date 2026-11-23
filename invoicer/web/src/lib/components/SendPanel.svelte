<!-- Sending an invoice (invoicer PLAN 3c, ARCHITECTURE §5.3, §3.4): its PDF,
     the "Mark as sent?" prompt a draft's export asks, the warning when a sent
     invoice changed since its PDF was kept, the PDFs kept so far, Share… and
     Copy file, and for a customer with a vendor portal its instructions, Open
     portal and copy buttons. Presentational: the page does the calls, so every
     action is a callback, and `busy` and `error` come back in. -->
<script lang="ts">
    import {
        anchorOf,
        defaultVia,
        exportPrompt,
        exportedText,
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
        onOpenPortal: () => void;
        onCopyText: (label: string, text: string) => void;
    }

    let {delivery, exported = null, busy = false, error = null, onExport, onSendAndExport, onShare, onCopyFile, onOpenPortal, onCopyText}: Props = $props();

    /** The prompt on screen, if any. */
    let asking = $state<"mark-sent" | "changed" | null>(null);
    let via = $state("");

    const caps = $derived(delivery.capabilities);
    const blocker = $derived(sendBlocker(delivery));
    const portal = $derived(delivery.portal);
    const viaChoice = $derived(via === "" ? defaultVia(delivery) : via);
    /** What Share…/Copy file act on: this session's export, else the kept PDF. */
    const target = $derived.by((): {destination: Destination; file: string} | null => {
        if (exported !== null) return {destination: exported.destination, file: exported.file};
        const latest = delivery.exports.find((e) => e.latest);
        return latest === undefined ? null : {destination: "kept", file: latest.file};
    });

    function clickExport(): void {
        const prompt = exportPrompt(delivery);
        if (prompt === "none") onExport({temp: false});
        else asking = prompt;
    }

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

<section class="flex flex-col gap-3" aria-labelledby="send-heading" data-testid="send-panel">
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
            <button type="button" class="btn btn-primary btn-sm" disabled={busy || delivery.status === null} onclick={clickExport} data-testid="send-export">
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
                <p class="mt-2 text-sm text-error" data-testid="send-blocker">{blocker}</p>
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

    {#if exported !== null}
        <div class="text-sm" role="status" data-testid="send-exported">
            {exportedText(exported)}
            {#if exported.earlier}<span class="block text-xs text-base-content/70">The earlier one is kept as {exported.earlier}.</span>{/if}
            {#each exported.warnings as warning, i (i)}
                <span class="block text-xs text-warning">{warning}</span>
            {/each}
        </div>
    {/if}

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

    {#if delivery.exports.length > 0}
        <div data-testid="send-kept">
            <div class="text-xs text-base-content/70">Kept in {delivery.pdf_dir}</div>
            <ul class="text-sm">
                {#each delivery.exports as pdf (pdf.file)}
                    <li class="flex justify-between gap-2" data-testid="send-kept-pdf">
                        <span class="truncate {pdf.latest ? 'font-medium' : 'text-base-content/70'}">{pdf.file}</span>
                        <span class="shrink-0 text-xs text-base-content/60 tabular-nums">{pdf.date ?? ""} · {sizeText(pdf.bytes)}</span>
                    </li>
                {/each}
            </ul>
        </div>
    {/if}
</section>
