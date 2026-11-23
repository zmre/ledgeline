// Sending an invoice and its PDF (invoicer PLAN 3c): the shapes of
// `GET`/`POST /api/invoices/{number}/pdf` and the pure rules the Send panel
// follows. The server decides where a PDF goes; the SPA only asks.

/** Where a PDF goes: kept in `pdf-dir` (a sent invoice), or a temporary file (a draft). */
export type Destination = "kept" | "temp" | "copy";

/** What this process can do natively; the panel hides what it can't. */
export interface Capabilities {
    pdf: boolean;
    share: boolean;
    copy_file: boolean;
    open_url: boolean;
}

/** A PDF kept for the invoice, newest first. */
export interface KeptPdf {
    file: string;
    kept_at: string | null;
    date: string | null;
    bytes: number;
    latest: boolean;
}

/** The customer's vendor portal (ARCHITECTURE §3.4). */
export interface Portal {
    name: string | null;
    url: string | null;
    login: string | null;
    supplier_id: string | null;
    requires_reference: boolean;
    instructions: string | null;
}

/** `GET /api/invoices/{number}/pdf`. */
export interface Delivery {
    number: string;
    document: string;
    status: "draft" | "sent" | "void" | null;
    sent_date: string | null;
    sent_via: string | null;
    destination: Destination;
    file: string;
    pdf_dir: string;
    sent_hash: string | null;
    changed_since_sent: boolean | null;
    exports: KeptPdf[];
    portal: Portal | null;
    copy: {number: string; reference: string | null; total: string | null};
    capabilities: Capabilities;
}

/** `POST /api/invoices/{number}/pdf`. */
export interface Exported {
    number: string;
    document: string;
    destination: Destination;
    file: string;
    kept_at?: string;
    earlier?: string;
    bytes: number;
    changed_since_sent: boolean | null;
    sent_hash?: "written" | "unchanged" | "stale";
    warnings: string[];
}

/** Where the Share… picker points from: the button, in CSS pixels. */
export interface Anchor {
    x: number;
    y: number;
    width: number;
    height: number;
}

/**
 * What clicking Export asks first:
 * - `mark-sent`: a draft ("Mark invoice 108 as sent?"; I2: sending books it);
 * - `changed`: a sent invoice edited since its PDF was kept (the PDF will differ);
 * - `none`: export straight away.
 */
export type ExportPrompt = "mark-sent" | "changed" | "none";

export function exportPrompt(delivery: Pick<Delivery, "status" | "changed_since_sent">): ExportPrompt {
    if (delivery.status === "draft") return "mark-sent";
    if (delivery.status === "sent" && delivery.changed_since_sent === true) return "changed";
    return "none";
}

/** How the invoice goes out, defaulting to the portal when the customer has one. */
export function defaultVia(delivery: Pick<Delivery, "portal" | "sent_via">): string {
    return delivery.sent_via ?? (delivery.portal !== null ? "portal" : "email");
}

/** The ways an invoice is delivered, offered when marking it sent. */
export const VIA_CHOICES: readonly string[] = ["email", "portal", "mail", "hand"];

/**
 * Why it can't be sent yet, or null: a portal that rejects invoices with no
 * `reference` (the server refuses too; this says so before the click).
 */
export function sendBlocker(delivery: Pick<Delivery, "portal" | "copy">): string | null {
    if (delivery.portal?.requires_reference && !delivery.copy.reference?.trim()) {
        return `${delivery.portal.name ?? "The customer's portal"} rejects invoices without a reference: add the PO as \`reference\` first.`;
    }
    return null;
}

/** A sentence for where an export went. */
export function exportedText(exported: Pick<Exported, "destination" | "kept_at" | "file">): string {
    switch (exported.destination) {
        case "kept":
            return `Saved ${exported.kept_at ?? exported.file}`;
        case "temp":
            return `Draft PDF ready: ${exported.file} (temporary, not kept)`;
        case "copy":
            return `Saved a copy: ${exported.file}`;
    }
}

/** `1234567` → "1.2 MB", for the kept list. */
export function sizeText(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** A DOM rect as the Share… anchor. */
export function anchorOf(rect: Pick<DOMRect, "left" | "top" | "width" | "height">): Anchor {
    return {x: rect.left, y: rect.top, width: rect.width, height: rect.height};
}
