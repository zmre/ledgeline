<script lang="ts">
    // Navbar problems indicator (WP-08): count badge colored by the worst
    // severity present; clicking toggles the problems drawer (the layout's
    // daisyUI drawer checkbox is bound to problems.drawerOpen).
    import {problems} from "$lib/stores/problems.svelte";

    const badgeClass = $derived(problems.maxSeverity === "error" ? "badge-error" : problems.maxSeverity === "warning" ? "badge-warning" : "badge-info");
    const label = $derived(problems.count === 1 ? "1 problem" : `${problems.count} problems`);
</script>

<button
    type="button"
    class="btn btn-ghost btn-sm btn-square indicator"
    aria-label={label}
    title={label}
    onclick={() => (problems.drawerOpen = !problems.drawerOpen)}
>
    {#if problems.count > 0}
        <span class="indicator-item badge badge-xs {badgeClass} font-semibold">{problems.count}</span>
    {/if}
    <!-- triangle-exclamation -->
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4" aria-hidden="true">
        <path
            stroke-linecap="round"
            stroke-linejoin="round"
            d="M12 9v4m0 4h.01M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"
        />
    </svg>
</button>
