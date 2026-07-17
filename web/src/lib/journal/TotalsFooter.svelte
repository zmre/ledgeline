<script lang="ts">
    // Journal footer (WP-03): the filtered transaction count and the active
    // period label only — the header owns any monetary figures.
    let {
        count,
        period,
    }: {
        count: number;
        period: string;
    } = $props();
</script>

<footer class="bg-base-200 rounded-box flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-2">
    <div class="text-base-content/70 flex items-center gap-3 text-sm">
        <span class="whitespace-nowrap"><span class="text-base-content font-medium">{count}</span> {count === 1 ? "transaction" : "transactions"}</span>
        <span class="whitespace-nowrap">{period}</span>
    </div>
</footer>
