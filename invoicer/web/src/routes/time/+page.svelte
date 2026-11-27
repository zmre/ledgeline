<!-- Time's home (plan 46 T-dash, W1–W3): the time dashboard. The component
     renders its own heading and width; this route only mounts it. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import TimeDashboard from "#lib/time/dashboard/TimeDashboard.svelte";
</script>

<svelte:head><title>Time · Ledgeline Invoicer</title></svelte:head>

<TimeDashboard entriesBase={resolve("/time/entries")} trackHref={resolve("/time/track")} />
