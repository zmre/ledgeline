<script lang="ts">
    // Column config dropdown (WP-03): gear icon toggling journal table columns,
    // persisted via settings.columns (localStorage).
    //
    // # Why `<details>` and not daisyUI's default dropdown
    //
    // daisyUI hides `.dropdown-content` with
    // `:not(details,.dropdown-open,.dropdown-hover:hover,:focus-within)`, so the
    // plain version opens on FOCUS and nothing else. This trigger is a real
    // `<button>`, and on macOS WebKit — Safari, and the WKWebView this app ships
    // in — clicking a button does not focus it. The menu therefore could not be
    // opened by mouse at all on that engine; daisyUI's own examples use
    // `<div tabindex="0" role="button">` precisely to dodge this, which is a
    // focusable div standing in for a button rather than a fix.
    //
    // `<details>` toggles on click in every engine, with no dependence on focus
    // behaviour, and daisyUI supports it explicitly (`&:is(details)`). It is
    // also the only variant whose open state is REAL DOM rather than a CSS
    // pseudo-class, which is what lets ColumnMenu.svelte.test.ts assert that the
    // menu opens — jsdom has no CSS engine and could never have caught this.
    import {settings, type ColumnConfig} from "$lib/stores/settings.svelte";

    const defs: {key: keyof ColumnConfig; label: string}[] = [
        {key: "date", label: "Date"},
        {key: "status", label: "Status"},
        {key: "description", label: "Description"},
        {key: "accounts", label: "Accounts"},
        {key: "amount", label: "Amount"},
    ];

    let open = $state(false);
    let root = $state<HTMLDetailsElement | null>(null);

    function toggle(key: keyof ColumnConfig): void {
        settings.columns = {...settings.columns, [key]: !settings.columns[key]};
    }

    // Dismissal, attached only while open so there is no idle document listener.
    $effect(() => {
        if (!open || root === null) return;
        const node = root;
        const onPointerDown = (event: PointerEvent): void => {
            if (!node.contains(event.target as Node)) open = false;
        };
        const onKeyDown = (event: KeyboardEvent): void => {
            if (event.key === "Escape") open = false;
        };
        document.addEventListener("pointerdown", onPointerDown, true);
        document.addEventListener("keydown", onKeyDown);
        return () => {
            document.removeEventListener("pointerdown", onPointerDown, true);
            document.removeEventListener("keydown", onKeyDown);
        };
    });
</script>

<details bind:this={root} bind:open class="dropdown dropdown-end">
    <summary class="btn btn-ghost btn-xs" aria-label="Configure columns" title="Configure columns">
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="h-4 w-4" aria-hidden="true">
            <path
                fill-rule="evenodd"
                d="M6.955 1.45c.083-.5.514-.867 1.02-.867h.05c.507 0 .938.368 1.02.868l.114.685c.06.366.31.669.645.826.334.158.723.153 1.037-.048l.585-.373a1.033 1.033 0 0 1 1.4.253l.03.04a1.034 1.034 0 0 1-.132 1.415l-.47.478c-.263.267-.36.65-.288 1.014.072.365.293.68.622.85l.616.316c.45.232.665.755.51 1.235l-.015.048a1.034 1.034 0 0 1-1.24.688l-.667-.174a1.098 1.098 0 0 0-1.01.24 1.098 1.098 0 0 0-.383.968l.073.685a1.034 1.034 0 0 1-.905 1.138l-.05.005a1.034 1.034 0 0 1-1.12-.827l-.135-.678a1.098 1.098 0 0 0-.663-.79 1.098 1.098 0 0 0-1.03.09l-.575.39a1.033 1.033 0 0 1-1.404-.226l-.031-.04a1.034 1.034 0 0 1 .105-1.417l.462-.487c.257-.272.347-.657.267-1.02a1.098 1.098 0 0 0-.638-.837l-.622-.304a1.034 1.034 0 0 1-.533-1.225l.014-.048c.146-.487.65-.775 1.146-.657l.67.161c.36.087.734-.019 1.005-.26.27-.24.402-.605.343-.972l-.11-.685ZM8 10.25a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5Z"
                clip-rule="evenodd"
            />
        </svg>
        Columns
    </summary>
    <ul class="dropdown-content menu bg-base-200 rounded-box border-base-300 z-20 w-44 border p-2 shadow-lg">
        {#each defs as def (def.key)}
            <li>
                <label class="label cursor-pointer justify-start gap-2 py-1">
                    <input type="checkbox" class="checkbox checkbox-xs" checked={settings.columns[def.key]} onchange={() => toggle(def.key)} />
                    <span class="label-text text-sm">{def.label}</span>
                </label>
            </li>
        {/each}
    </ul>
</details>
