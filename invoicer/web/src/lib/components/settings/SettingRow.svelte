<!-- One setting on the Settings page (SETTINGS-PLAN §4): its label and one
     help sentence, the control, and what the value is: "Default" when the
     books don't set it, "Found for you: …" for an account the books found,
     "Use the default" when they set it. "Affects new bookings" (text, not
     colour alone) marks a setting Save asks to confirm. The setting's name
     is a lookup key only; the row shows its label.

     One column below `lg`; from `lg` the label and help on the left, the
     control on the right (container queries, so 200% behaves like a narrow
     window). -->
<script lang="ts">
    import type {Snippet} from "svelte";
    import {SETTINGS, settingLabel} from "#lib/labels.js";

    let {
        setting,
        isDefault = false,
        found = null,
        offersDefault = false,
        affects = false,
        changed = false,
        problem = null,
        disabled = false,
        onUseDefault,
        control,
        extra,
    }: {
        setting: string;
        isDefault?: boolean;
        /** The account the books found, shown when the setting is unset. */
        found?: string | null;
        offersDefault?: boolean;
        affects?: boolean;
        changed?: boolean;
        problem?: string | null;
        disabled?: boolean;
        onUseDefault?: () => void;
        /** The control, given the id its label points at. */
        control: Snippet<[string]>;
        /** Under the control: a pin, the next number. */
        extra?: Snippet;
    } = $props();

    const uid = $props.id();
    const controlId = `${uid}-control`;
    const help = $derived(Object.hasOwn(SETTINGS, setting) ? SETTINGS[setting].help : "");
    const testid = $derived(`setting-${setting.replace(/[^a-z0-9]+/gi, "-")}`);
</script>

<div class="grid gap-x-6 gap-y-2 border-b border-base-300 py-3 last:border-b-0 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]" data-testid={testid}>
    <div class="flex min-w-0 flex-col gap-1">
        <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <label for={controlId} class="font-medium" id="{uid}-label">{settingLabel(setting)}</label>
            {#if changed}<span class="text-xs text-base-content/60" data-testid="{testid}-changed">Changed</span>{/if}
            {#if affects}
                <span class="badge badge-outline badge-sm badge-warning" data-testid="{testid}-affects">Affects new bookings</span>
            {/if}
        </div>
        <p class="text-sm text-base-content/70" id="{uid}-help">{help}</p>
    </div>
    <div class="flex min-w-0 flex-col gap-1.5">
        {@render control(controlId)}
        <div class="flex flex-wrap items-center gap-2 text-xs">
            {#if isDefault && found !== null}
                <span class="badge badge-ghost badge-sm" data-testid="{testid}-found">Found for you: {found}</span>
            {:else if isDefault}
                <span class="badge badge-ghost badge-sm" data-testid="{testid}-default">Default</span>
            {/if}
            {#if offersDefault && onUseDefault !== undefined}
                <button type="button" class="btn btn-link px-0 btn-xs" {disabled} onclick={onUseDefault} data-testid="{testid}-use-default"
                    >Use the default</button
                >
            {/if}
        </div>
        {#if problem !== null}
            <p class="text-sm text-error-text" role="alert" data-testid="{testid}-problem">{problem}</p>
        {/if}
        {@render extra?.()}
    </div>
</div>
