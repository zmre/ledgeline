<script lang="ts">
    // Navbar problems indicator (WP-08): count badge colored by the worst
    // severity present; clicking toggles the problems drawer (the layout's
    // daisyUI drawer checkbox is bound to problems.drawerOpen). Hidden entirely
    // when there is nothing to report: a clean journal shouldn't wear a warning sign.
    //
    // Bank sync's problems (plan 43) count here too, and the drawer lists them
    // as their own "Bank sync" group: they come from the sync status, not from
    // `/api/diagnostics`.
    import {problems} from "#lib/stores/problems.svelte.js";
    import {syncStore} from "#lib/sync/syncStore.svelte.js";

    const count = $derived(problems.count + syncStore.problems.length);
    const worst = $derived(
        problems.maxSeverity === "error" || syncStore.maxSeverity === "error"
            ? "error"
            : problems.maxSeverity === "warning" || syncStore.maxSeverity === "warning"
              ? "warning"
              : "info"
    );
    const badgeClass = $derived(worst === "error" ? "badge-error" : worst === "warning" ? "badge-warning" : "badge-info");
    const label = $derived(count === 1 ? "1 problem" : `${count} problems`);
</script>

{#if count > 0}
    <button
        type="button"
        class="btn indicator btn-square btn-ghost btn-sm"
        aria-label={label}
        title={label}
        onclick={() => (problems.drawerOpen = !problems.drawerOpen)}
    >
        <span class="indicator-item badge badge-xs {badgeClass} font-semibold">{count}</span>
        <!-- triangle-exclamation -->
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4" aria-hidden="true">
            <path
                stroke-linecap="round"
                stroke-linejoin="round"
                d="M12 9v4m0 4h.01M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z"
            />
        </svg>
    </button>
{/if}
