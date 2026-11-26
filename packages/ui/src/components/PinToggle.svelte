<!-- "Use for this ledger only" / "Use for these books only" (plan 39): keep one
     shared preference for the open books alone, in their own settings file.
     The same checkbox everywhere a shared setting can be pinned: the theme,
     the text size, Ledgeline's Books panel and the invoicer's Settings. The
     caller says whether it is pinned and does the pinning; this only draws
     it, and stays disabled while a pin is under way. -->
<script lang="ts">
    let {
        pinned,
        busy = false,
        label = "Use for this ledger only",
        onChange,
        testid,
    }: {
        pinned: boolean;
        busy?: boolean;
        label?: string;
        onChange: (pinned: boolean) => void;
        testid?: string;
    } = $props();
</script>

<label class="flex cursor-pointer items-center gap-2 text-sm">
    <input
        type="checkbox"
        class="checkbox checkbox-sm"
        checked={pinned}
        disabled={busy}
        onchange={(e) => onChange(e.currentTarget.checked)}
        data-testid={testid}
    />
    {label}
</label>
