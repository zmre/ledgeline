<!-- The sidebar's lamp (plan 46 T-light, T17), above the section links on
     every screen. Running: a green ▶, the account, the live elapsed time and
     its key; click stops it. Stopped: a grey ■, the last account and the key
     that resumes it; click resumes. No red anywhere, and colour is never the
     only signal: the shape, the words in the accessible name ("running",
     "stopped") and the elapsed time carry it too.

     Before time tracking is set up: one quiet link to Track, which sets it
     up. Nothing at all while the state is unknown (no books, a failed read).
     A write that didn't go through says so under the lamp, quietly. -->
<script lang="ts">
    import {IS_MAC} from "#ui/keys/chord.js";
    import {elapsedText, slotHint} from "./format.js";
    import type {Tracker} from "./tracker.svelte.js";

    let {tracker, trackHref, mac = IS_MAC}: {tracker: Tracker; trackHref: string; mac?: boolean} = $props();

    const state = $derived(tracker.state);
    const running = $derived(state?.running ?? null);
    const last = $derived(state?.last ?? null);
    const elapsed = $derived(elapsedText(tracker.elapsed));
    const runningHint = $derived(running?.slot != null ? slotHint(running.slot, mac) : null);
    const resumeHint = $derived(last?.slot != null ? slotHint(last.slot, mac) : null);

    function click(): void {
        if (running !== null) void tracker.stop();
        else if (last !== null) {
            if (last.slot !== null && tracker.slot(last.slot)?.account === last.account) void tracker.toggle(last.slot);
            else void tracker.toggleAccount(last.account, last.label);
        }
    }
</script>

{#if state !== null}
    <div class="px-2 pt-1 md:pt-0 print:hidden" data-testid="time-lamp">
        {#if !state.configured}
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves it -->
            <a
                href={trackHref}
                class="block rounded-field px-2 py-1 text-xs text-base-content/60 hover:bg-base-300 hover:text-base-content"
                data-testid="time-lamp-setup">Set up time tracking</a
            >
            <!-- eslint-enable svelte/no-navigation-without-resolve -->
        {:else if running !== null}
            <button
                type="button"
                class="flex w-full items-center gap-2 rounded-field border border-success/40 bg-success/10 px-2 py-1.5 text-left text-sm hover:bg-success/20"
                aria-label="Time tracking running: {running.label}, {elapsed}. Stop"
                title="Stop {running.label}"
                onclick={click}
                data-testid="time-lamp-running"
            >
                <span class="text-success" aria-hidden="true">▶</span>
                <span class="flex min-w-0 flex-1 flex-col">
                    <span class="truncate font-medium">{running.label}</span>
                    {#if runningHint !== null}<span class="text-xs text-base-content/60">{runningHint}</span>{/if}
                </span>
                <span class="tabular-nums" data-testid="time-lamp-elapsed">{elapsed}</span>
            </button>
        {:else if last !== null}
            <button
                type="button"
                class="flex w-full items-center gap-2 rounded-field border border-base-300 px-2 py-1.5 text-left text-sm text-base-content/70 hover:bg-base-300"
                aria-label="Time tracking stopped. Resume {last.label}"
                title="Resume {last.label}"
                onclick={click}
                data-testid="time-lamp-stopped"
            >
                <span class="text-base-content/50" aria-hidden="true">■</span>
                <span class="flex min-w-0 flex-1 flex-col">
                    <span class="truncate">{last.label}</span>
                    <span class="text-xs text-base-content/50" data-testid="time-lamp-hint"
                        >{resumeHint === null ? "Click to resume" : `${resumeHint} to resume`}</span
                    >
                </span>
            </button>
        {:else}
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the layout resolves it -->
            <a
                href={trackHref}
                class="flex items-center gap-2 rounded-field border border-base-300 px-2 py-1.5 text-sm text-base-content/70 hover:bg-base-300"
                aria-label="Time tracking stopped. Open Track"
                data-testid="time-lamp-idle"
            >
                <span class="text-base-content/50" aria-hidden="true">■</span>
                <span>Not tracking</span>
            </a>
            <!-- eslint-enable svelte/no-navigation-without-resolve -->
        {/if}
        {#if tracker.message !== null}
            <p class="mt-1 px-1 text-xs text-base-content/70" role="status" data-testid="time-lamp-message">{tracker.message}</p>
        {/if}
    </div>
{/if}
