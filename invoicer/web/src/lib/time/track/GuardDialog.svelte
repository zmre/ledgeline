<!-- The long-running guard (plan 46 T-running): a timer older than the
     threshold, or started on an earlier day, asks what to do with it. Keep
     all · Stop at the last activity (when one is known) · Stop at… a time
     you give · Discard. Each maps onto `POST /api/time/stop {at, discard}`;
     Keep all only stops asking for this timer. Escape is Keep all: nothing
     is lost by it. -->
<script lang="ts">
    import {dismissible} from "#ui/keys/dismissible.js";
    import {modalKeys} from "#lib/keys/modalKeys.svelte.js";
    import type {Guard, RunningView} from "#lib/time/types.js";
    import {addMinutes, clockOf, dayHeading, durationText} from "./format.js";

    let {
        guard,
        running,
        busy = false,
        onKeep,
        onStopAt,
        onDiscard,
    }: {
        guard: Guard;
        running: RunningView;
        busy?: boolean;
        onKeep: () => void;
        /** `YYYY-MM-DDTHH:MM:SS`. */
        onStopAt: (at: string) => void;
        onDiscard: () => void;
    } = $props();

    modalKeys("time-guard");

    const uid = $props.id();
    let choosing = $state(false);
    let at = $state("");
    let invalid = $state<string | null>(null);

    const sinceDay = $derived(running.since.slice(0, 10));
    const started = $derived(`${dayHeading(sinceDay)} at ${clockOf(running.since)}`);

    function openStopAt(): void {
        // Start from the threshold after the start (an hour, with no threshold).
        at = addMinutes(running.since, guard.threshold_minutes > 0 ? guard.threshold_minutes : 60);
        invalid = null;
        choosing = true;
    }

    function confirmStopAt(): void {
        const moment = at.length === 16 ? `${at}:00` : at;
        if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/.test(moment)) {
            invalid = "Give a date and time.";
            return;
        }
        if (moment <= running.since) {
            invalid = `Pick a time after it started (${started}).`;
            return;
        }
        onStopAt(moment);
    }
</script>

<div class="modal modal-open" role="presentation">
    <div
        class="modal-box"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="{uid}-title"
        aria-describedby="{uid}-text"
        data-testid="time-guard"
        use:dismissible={{onDismiss: onKeep, trap: true, initial: "[data-safe-choice]"}}
    >
        <h2 id="{uid}-title" class="text-lg font-semibold">Still working on {running.label}?</h2>
        <p id="{uid}-text" class="mt-2 text-sm">
            {#if guard.previous_day}
                This timer started on an earlier day, {started}, and has run for {durationText(guard.age.seconds)}.
            {:else}
                This timer has run for {durationText(guard.age.seconds)}, since {clockOf(running.since)}.
            {/if}
            Was it left running?
        </p>
        {#if choosing}
            <div class="mt-3 flex flex-col gap-2">
                <label class="text-sm font-medium" for="{uid}-at">Stop it at</label>
                <input id="{uid}-at" class="input w-full" type="datetime-local" bind:value={at} min={running.since.slice(0, 16)} data-testid="time-guard-at" />
                {#if invalid !== null}<p class="text-sm" role="alert">{invalid}</p>{/if}
                <div class="flex justify-end gap-2">
                    <button type="button" class="btn btn-ghost btn-sm" onclick={() => (choosing = false)} disabled={busy}>Back</button>
                    <button type="button" class="btn btn-primary btn-sm" onclick={confirmStopAt} disabled={busy} data-testid="time-guard-at-confirm"
                        >Stop then</button
                    >
                </div>
            </div>
        {:else}
            <div class="mt-4 flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:justify-end">
                <button type="button" class="btn btn-sm" onclick={onKeep} disabled={busy} data-safe-choice data-testid="time-guard-keep">Keep all</button>
                {#if guard.last_activity !== null}
                    <button
                        type="button"
                        class="btn btn-sm"
                        onclick={() => onStopAt(guard.last_activity as string)}
                        disabled={busy}
                        data-testid="time-guard-last"
                    >
                        Stop at last activity ({clockOf(guard.last_activity)})
                    </button>
                {/if}
                <button type="button" class="btn btn-sm" onclick={openStopAt} disabled={busy} data-testid="time-guard-stop-at">Stop at…</button>
                <button type="button" class="btn btn-sm btn-warning" onclick={onDiscard} disabled={busy} data-testid="time-guard-discard">Discard</button>
            </div>
        {/if}
    </div>
</div>
