<script lang="ts">
    // The review settings (plan 32 D19), edited where they are used: the ▾ on
    // the journal's Review Recents button and the overlay's ⋯ menu. Per
    // browser (`settings.svelte.ts`). A number is committed on change, clamped
    // by the store; the journal's badges and stripes follow at once.
    import {settings} from "#lib/stores/settings.svelte.js";

    let {onChange}: {onChange?: () => void} = $props();

    const uid = $props.id();

    function commit(field: "reviewRecentLimit" | "reviewMaxDays", raw: string): void {
        const value = Number(raw);
        if (raw.trim() === "" || !Number.isFinite(value)) return;
        settings[field] = value;
        onChange?.();
    }
</script>

<div class="flex w-72 flex-col gap-3 p-1 text-sm" data-testid="review-settings">
    <h3 class="text-xs font-medium tracking-wide text-base-content/55 uppercase">Review settings</h3>
    <label class="flex items-center justify-between gap-3" for="{uid}-limit">
        <span>Review Recents takes</span>
        <span class="flex items-center gap-1.5">
            <input
                id="{uid}-limit"
                type="number"
                min="1"
                max="1000"
                class="input w-20 text-right tabular-nums input-xs"
                value={settings.reviewRecentLimit}
                onchange={(event) => commit("reviewRecentLimit", event.currentTarget.value)}
                aria-label="How many recent transactions to review"
            />
            <span class="text-base-content/60">at most</span>
        </span>
    </label>
    <label class="flex items-center justify-between gap-3" for="{uid}-days">
        <span>Look back</span>
        <span class="flex items-center gap-1.5">
            <input
                id="{uid}-days"
                type="number"
                min="0"
                max="36500"
                class="input w-20 text-right tabular-nums input-xs"
                value={settings.reviewMaxDays}
                onchange={(event) => commit("reviewMaxDays", event.currentTarget.value)}
                aria-label="How many days back to review"
            />
            <span class="text-base-content/60">days</span>
        </span>
    </label>
    <p class="-mt-2 text-xs text-base-content/50">0 days means no limit. Both queues use this window.</p>
    <label class="flex cursor-pointer items-center justify-between gap-3">
        <span>Mark unreviewed rows in the journal</span>
        <input
            type="checkbox"
            class="toggle toggle-sm"
            checked={settings.reviewMarkers}
            onchange={(event) => {
                settings.reviewMarkers = event.currentTarget.checked;
                onChange?.();
            }}
            aria-label="Mark unreviewed rows in the journal"
        />
    </label>
</div>
