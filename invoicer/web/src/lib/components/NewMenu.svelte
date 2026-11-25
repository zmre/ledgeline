<!-- The sidebar's New split button (Phase 4 feedback H20): the main part
     makes the first choice (New invoice); the arrow opens a menu of every
     choice. Links arrive resolved, as the sidebar's do.

     Keyboard (WAI-ARIA menu button): ArrowDown on either part, or Enter or
     Space on the arrow, opens the menu on its first item (ArrowUp on its
     last); in the menu ArrowUp/ArrowDown move (wrapping), Home/End jump,
     Enter follows the item, Escape closes it and puts focus back on the
     arrow, Tab closes it and moves on. A click outside closes it too.
     Shift+N, anywhere (PLAN 5k), opens it on its first item. -->
<script lang="ts" module>
    /** One thing the menu can make, its link already resolved. */
    export interface NewChoice {
        id: string;
        label: string;
        href: string;
    }
</script>

<script lang="ts">
    import {tick} from "svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";

    let {choices, label = "New"}: {choices: NewChoice[]; label?: string} = $props();

    let open = $state(false);
    let root = $state<HTMLElement | null>(null);
    let toggle = $state<HTMLButtonElement | null>(null);
    let menu = $state<HTMLUListElement | null>(null);

    const main = $derived(choices[0] ?? null);
    const menuId = $props.id();

    const items = (): HTMLAnchorElement[] => [...(menu?.querySelectorAll<HTMLAnchorElement>("a[role=menuitem]") ?? [])];

    async function show(at: "first" | "last" = "first"): Promise<void> {
        open = true;
        await tick();
        const all = items();
        (at === "first" ? all[0] : all[all.length - 1])?.focus();
    }

    registerKeys({
        id: "new-menu",
        bindings: [{keys: "N", label: "Open the New menu", group: "Global", enabled: () => main !== null, run: () => void show("first")}],
    });

    function close(refocus: boolean): void {
        open = false;
        if (refocus) toggle?.focus();
    }

    function onTriggerKeydown(event: KeyboardEvent): void {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            void show(event.key === "ArrowDown" ? "first" : "last");
        }
    }

    function onToggleKeydown(event: KeyboardEvent): void {
        if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            if (open) close(true);
            else void show("first");
            return;
        }
        onTriggerKeydown(event);
    }

    function onMenuKeydown(event: KeyboardEvent): void {
        const all = items();
        const at = all.indexOf(document.activeElement as HTMLAnchorElement);
        const go = (index: number): void => {
            event.preventDefault();
            all[(index + all.length) % all.length]?.focus();
        };
        switch (event.key) {
            case "ArrowDown":
                go(at + 1);
                break;
            case "ArrowUp":
                go(at < 0 ? all.length - 1 : at - 1);
                break;
            case "Home":
                go(0);
                break;
            case "End":
                go(all.length - 1);
                break;
            case "Escape":
                event.preventDefault();
                event.stopPropagation();
                close(true);
                break;
            case "Tab":
                close(false);
                break;
            case " ":
                // Space follows the item, as Enter does on a link.
                event.preventDefault();
                (document.activeElement as HTMLElement | null)?.click();
                break;
        }
    }

    function onWindowPointer(event: PointerEvent): void {
        if (open && root !== null && !root.contains(event.target as Node)) close(false);
    }
</script>

<svelte:window onpointerdown={onWindowPointer} />

{#if main !== null}
    <div class="relative" bind:this={root} data-testid="new-menu">
        <div class="join flex w-full">
            <!-- eslint-disable svelte/no-navigation-without-resolve -- the sidebar's links arrive resolved -->
            <a href={main.href} class="btn join-item grow btn-primary btn-sm" onkeydown={onTriggerKeydown} data-testid="new-main">{main.label}</a>
            <button
                bind:this={toggle}
                type="button"
                class="btn join-item border-l border-l-primary-content/25 px-2 btn-primary btn-sm"
                aria-label="{label}: more choices"
                aria-haspopup="menu"
                aria-expanded={open}
                aria-controls={open ? menuId : undefined}
                onclick={() => (open ? close(false) : void show("first"))}
                onkeydown={onToggleKeydown}
                data-testid="new-toggle"
            >
                <svg viewBox="0 0 16 16" class="h-3.5 w-3.5" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2"
                    ><path d="M4 6l4 4 4-4" stroke-linecap="round" stroke-linejoin="round" /></svg
                >
            </button>
        </div>
        {#if open}
            <ul
                bind:this={menu}
                id={menuId}
                role="menu"
                aria-label={label}
                tabindex="-1"
                class="menu absolute top-full right-0 z-30 mt-1 w-52 rounded-box border border-base-300 bg-base-100 p-1 shadow-lg md:right-auto md:left-0 md:w-full"
                onkeydown={onMenuKeydown}
                data-testid="new-menu-list"
            >
                {#each choices as choice (choice.id)}
                    <li role="none">
                        <a role="menuitem" tabindex="-1" href={choice.href} onclick={() => close(false)} data-testid="new-menu-{choice.id}">{choice.label}</a>
                    </li>
                {/each}
            </ul>
        {/if}
        <!-- eslint-enable svelte/no-navigation-without-resolve -->
    </div>
{/if}
