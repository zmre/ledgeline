<script lang="ts">
    import "../app.css";
    import favicon from "$lib/assets/favicon.svg";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";

    let {children} = $props();
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>

<div class="bg-base-100 text-base-content flex min-h-screen flex-col">
    <header class="navbar bg-base-200 min-h-12 shadow-sm">
        <div class="navbar-start">
            <a href={resolve("/")} class="btn btn-ghost px-2 text-lg font-semibold tracking-tight">Ledgeline</a>
        </div>
        <nav class="navbar-center">
            <ul class="menu menu-horizontal gap-1 px-1">
                <li>
                    <a href={resolve("/")} class={page.url.pathname === "/" ? "menu-active" : ""}>Journal</a>
                </li>
                <li>
                    <a href={resolve("/reports")} class={page.url.pathname.startsWith("/reports") ? "menu-active" : ""}>Reports</a>
                </li>
            </ul>
        </nav>
        <div class="navbar-end pr-2">
            <!-- Connection-status indicator mounts here (WP-02). -->
            <span id="connection-status"></span>
        </div>
    </header>

    <main class="mx-auto w-full max-w-7xl grow p-4">
        {@render children()}
    </main>
</div>
