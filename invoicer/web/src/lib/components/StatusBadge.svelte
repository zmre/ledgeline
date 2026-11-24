<!-- Where a document stands, as one badge: Draft, Open, Overdue, Partly
     paid, Paid or Void (`standing` in #lib/invoices.ts decides). -->
<script lang="ts">
    import {standing, type InvoiceEntry} from "#lib/invoices.js";

    let {doc}: {doc: Pick<InvoiceEntry, "status" | "paid" | "open" | "overdue">} = $props();

    const shown = $derived(standing(doc));
    const toneClass: Record<string, string> = {
        neutral: "badge-ghost",
        info: "badge-info",
        success: "badge-success",
        warning: "badge-warning",
        error: "badge-error",
    };
</script>

<span class="badge badge-sm whitespace-nowrap {toneClass[shown.tone]}" data-testid="status-badge">{shown.label}</span>
