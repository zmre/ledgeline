<script lang="ts">
    // The review settings (plan 32 D19), edited where they are used: the ▾ on
    // the journal's Review Recents button and the overlay's ⋯ menu. Per
    // browser (`settings.svelte.ts`). A number is committed on change, clamped
    // by the store; the journal's badges and stripes follow at once.
    //
    // Find Matches' settings (plan 35 D8) live here too: Review Recents' ▾
    // shows both groups, Find Matches' ▾ (Amendment A1) and each overlay's ⋯
    // only their own (`show`). The lookback has presets beside its number.
    import {LOOKBACK_PRESETS} from "#lib/matching/model.js";
    import {settings} from "#lib/stores/settings.svelte.js";

    let {onChange, show = "all"}: {onChange?: () => void; show?: "all" | "review" | "matches"} = $props();

    const uid = $props.id();

    type NumberSetting = "reviewRecentLimit" | "reviewMaxDays" | "matchWindow" | "matchLookback" | "matchTransitDays";

    function commit(field: NumberSetting, raw: string): void {
        const value = Number(raw);
        if (raw.trim() === "" || !Number.isFinite(value)) return;
        settings[field] = value;
        onChange?.();
    }
</script>

{#snippet days(field: NumberSetting, label: string, aria: string, min: number, max: number)}
    <label class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1" for="{uid}-{field}">
        <span>{label}</span>
        <span class="flex items-center gap-1.5">
            <input
                id="{uid}-{field}"
                type="number"
                {min}
                {max}
                class="input w-20 text-right tabular-nums input-xs"
                value={settings[field]}
                onchange={(event) => commit(field, event.currentTarget.value)}
                aria-label={aria}
                data-testid="setting-{field}"
            />
            <span class="text-base-content/60">days</span>
        </span>
    </label>
{/snippet}

<div class="flex w-72 max-w-full flex-col gap-3 p-1 text-sm" data-testid="review-settings">
    {#if show !== "matches"}
        <h3 class="text-xs font-medium tracking-wide text-base-content/55 uppercase">Review settings</h3>
        <label class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1" for="{uid}-limit">
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
        <label class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1" for="{uid}-days">
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
        <label class="flex cursor-pointer flex-wrap items-center justify-between gap-x-3 gap-y-1">
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
    {/if}
    {#if show !== "review"}
        <h3 class={["text-xs font-medium tracking-wide text-base-content/55 uppercase", show === "all" && "mt-1 border-t border-base-content/10 pt-3"]}>
            Find Matches
        </h3>
        {@render days("matchWindow", "Halves at most", "How many days apart the two halves of a transfer may be", 1, 60)}
        {@render days("matchLookback", "Look back", "How many days back to look for matches", 0, 36500)}
        <div class="-mt-1 flex flex-wrap justify-end gap-1" role="group" aria-label="Look back presets">
            {#each LOOKBACK_PRESETS as preset (preset.days)}
                <button
                    type="button"
                    class={["btn btn-xs", settings.matchLookback === preset.days ? "btn-active btn-primary" : "btn-ghost"]}
                    aria-pressed={settings.matchLookback === preset.days}
                    onclick={() => {
                        settings.matchLookback = preset.days;
                        onChange?.();
                    }}
                    data-testid="lookback-preset-{preset.days}">{preset.label}</button
                >
            {/each}
        </div>
        <p class="-mt-2 text-xs text-base-content/50">0 days means no limit.</p>
        {@render days("matchTransitDays", "In transit after", "Days before an unmatched transfer is flagged as in transit", 1, 365)}
    {/if}
</div>
