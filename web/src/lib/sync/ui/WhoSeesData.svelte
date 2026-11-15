<!-- "Who sees your bank data" (plan 43 §2): an expandable table, in the wizard
     and behind each connection card's ⓘ. A native <details>, so it opens from
     the keyboard and needs no state. -->
<script lang="ts">
    let {summary = "Who sees your data", open = false}: {summary?: string; open?: boolean} = $props();
</script>

<details class="text-sm" {open} data-testid="sync-who-sees">
    <summary class="cursor-pointer text-base-content/70">{summary} ⓘ</summary>
    <div class="mt-2 overflow-x-auto">
        <table class="table table-xs">
            <thead>
                <tr><th>Connection</th><th>Who sees your transactions</th><th>Where your bank password lives</th></tr>
            </thead>
            <tbody>
                <tr><td>SimpleFIN Bridge</td><td>SimpleFIN Bridge and MX (its data provider)</td><td>With MX, not with Ledgeline</td></tr>
                <tr><td>Manual download</td><td>Nobody but you</td><td>Nowhere (you log in yourself)</td></tr>
            </tbody>
        </table>
    </div>
</details>
