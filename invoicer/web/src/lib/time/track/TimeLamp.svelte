<!-- The sidebar's lamp (plan 46 T-light, T17), above the section links on
     every screen. Running: a green ▶, the account, the live elapsed time and
     its key. Stopped: a grey ■, the last account and the key that resumes
     it. No red anywhere, and colour is never the only signal: the shape, the
     words in the accessible name ("running", "stopped") and the elapsed time
     carry it too.

     The lamp is the tracker: a click, or `T` from any screen, opens the
     panel beside it (LampPanel: the slots, ±step, the note, Open Track);
     `T` again, Escape or a click outside closes it, and focus comes back to
     the lamp. The lamp's wrapper is the panel's dismissible node, so a click
     on the lamp itself closes the panel rather than reopening it.

     Before time tracking is set up the lamp is one quiet "Set up time
     tracking", and its panel points to Track's setup. Nothing at all while
     the state is unknown (no books, a failed read). A write that didn't go
     through says so under the lamp, quietly. -->
<script lang="ts">
    import {tick} from "svelte";
    import {IS_MAC} from "#ui/keys/chord.js";
    import {dismissible} from "#ui/keys/dismissible.js";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import {PRIORITY} from "#ui/keys/types.js";
    import {elapsedText, slotHint} from "./format.js";
    import LampPanel from "./LampPanel.svelte";
    import type {Tracker} from "./tracker.svelte.js";

    let {
        tracker,
        trackHref,
        onPick = () => {},
        mac = IS_MAC,
    }: {
        tracker: Tracker;
        trackHref: string;
        /** An empty slot 1–9 pressed in the panel: Track, its picker open for that slot. */
        onPick?: (key: number) => void;
        mac?: boolean;
    } = $props();

    const uid = $props.id();
    const shown = $derived(tracker.state);
    const running = $derived(shown?.running ?? null);
    const last = $derived(shown?.last ?? null);
    const elapsed = $derived(elapsedText(tracker.elapsed));
    const runningHint = $derived(running?.slot != null ? slotHint(running.slot, mac) : null);
    const resumeHint = $derived(last?.slot != null ? slotHint(last.slot, mac) : null);

    let open = $state(false);
    let lampButton = $state<HTMLButtonElement | null>(null);

    function show(): void {
        // The opener dismissible restores to, whichever way the panel opened.
        lampButton?.focus();
        open = true;
    }

    /** Close, focus back on the lamp (a click elsewhere then takes it, as clicks do). */
    function close(): void {
        if (!open) return;
        open = false;
        void tick().then(() => lampButton?.focus());
    }

    function toggle(): void {
        if (open) close();
        else show();
    }

    // The panel is gone with the state (the books closed).
    $effect(() => {
        if (shown === null) open = false;
    });

    registerKeys({
        id: "invoicer-time-lamp",
        priority: PRIORITY.page,
        bindings: [{keys: "T", label: "Open or close the time tracker", group: "Time", enabled: () => tracker.state !== null, run: toggle}],
    });
</script>

{#if shown !== null}
    <div class="relative px-2 pt-1 md:pt-0 print:hidden" data-testid="time-lamp" use:dismissible={{onDismiss: () => close(), active: open, outside: true}}>
        <button
            bind:this={lampButton}
            type="button"
            class="flex w-full items-center gap-2 rounded-field px-2 py-1.5 text-left text-sm
                {!shown.configured
                ? 'py-1 text-xs text-base-content/60 hover:bg-base-300 hover:text-base-content'
                : running !== null
                  ? 'border border-success/40 bg-success/10 hover:bg-success/20'
                  : 'border border-base-300 text-base-content/70 hover:bg-base-300'}"
            aria-label={!shown.configured
                ? "Set up time tracking"
                : running !== null
                  ? `Time tracking running: ${running.label}, ${elapsed}`
                  : last !== null
                    ? `Time tracking stopped, last ${last.label}`
                    : "Time tracking stopped"}
            aria-haspopup="dialog"
            aria-expanded={open}
            aria-controls={open ? `${uid}-panel` : undefined}
            title="Time tracker (T)"
            onclick={toggle}
            data-testid={!shown.configured
                ? "time-lamp-setup"
                : running !== null
                  ? "time-lamp-running"
                  : last !== null
                    ? "time-lamp-stopped"
                    : "time-lamp-idle"}
        >
            {#if !shown.configured}
                <span>Set up time tracking</span>
            {:else if running !== null}
                <span class="text-success" aria-hidden="true">▶</span>
                <span class="flex min-w-0 flex-1 flex-col">
                    <span class="truncate font-medium">{running.label}</span>
                    {#if runningHint !== null}<span class="text-xs text-base-content/60">{runningHint}</span>{/if}
                </span>
                <span class="tabular-nums" data-testid="time-lamp-elapsed">{elapsed}</span>
            {:else if last !== null}
                <span class="text-base-content/50" aria-hidden="true">■</span>
                <span class="flex min-w-0 flex-1 flex-col">
                    <span class="truncate">{last.label}</span>
                    {#if resumeHint !== null}
                        <span class="text-xs text-base-content/50" data-testid="time-lamp-hint">{resumeHint} to resume</span>
                    {/if}
                </span>
            {:else}
                <span class="text-base-content/50" aria-hidden="true">■</span>
                <span>Not tracking</span>
            {/if}
        </button>
        {#if open}
            <LampPanel {tracker} {trackHref} id="{uid}-panel" {onPick} onClose={() => close()} />
        {/if}
        {#if tracker.message !== null && !open}
            <p class="mt-1 px-1 text-xs text-base-content/70" role="status" data-testid="time-lamp-message">{tracker.message}</p>
        {/if}
    </div>
{/if}
