<!-- Time's home (plan 46 T-dash, W1–W3): the Overview tab, the time
     dashboard. The component renders the Time heading, its tabs and its
     width; this route only mounts it. -->
<script lang="ts">
    import {resolve} from "$app/paths";
    import TimeDashboard from "#lib/time/dashboard/TimeDashboard.svelte";
</script>

<svelte:head><title>Time · Ledgeline Invoicer</title></svelte:head>

<TimeDashboard entriesBase={resolve("/time/entries")} trackHref={resolve("/time/track")} />
