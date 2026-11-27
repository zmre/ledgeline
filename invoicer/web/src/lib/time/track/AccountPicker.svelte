<!-- Choose an account (plan 46 T-slots): slot 0's Other…, and a pin's
     editor. Fuzzy search as Ledgeline's account field matches (segment
     aware); with nothing typed, the current choice first, then recent
     accounts, then the rest A–Z. Create account… sits at the bottom when
     what's typed isn't an account yet. A pin can also be cleared.

     ↑/↓ move, Enter chooses, Escape cancels; the page beneath has no keys
     while it's open. -->
<script lang="ts">
    import {dismissible} from "#ui/keys/dismissible.js";
    import {modalKeys} from "#lib/keys/modalKeys.svelte.js";
    import {accountLabel} from "./format.js";
    import {pickerOptions, type PickerOption} from "./picker.js";

    let {
        title,
        names,
        recents = [],
        current = null,
        allowCreate = true,
        onClear,
        error = null,
        busy = false,
        onChoose,
        onCancel,
    }: {
        title: string;
        /** Every account; null while they load. */
        names: string[] | null;
        recents?: string[];
        current?: string | null;
        allowCreate?: boolean;
        /** A pin can be cleared; absent, no Clear button. */
        onClear?: () => void;
        error?: string | null;
        busy?: boolean;
        /** `created`: the account is new (declare it first). */
        onChoose: (account: string, created: boolean) => void;
        onCancel: () => void;
    } = $props();

    modalKeys("time-account-picker");

    const uid = $props.id();
    let query = $state("");
    let active = $state(0);

    const options = $derived(pickerOptions(query, names ?? [], recents, current, allowCreate));
    const shown = $derived(options.slice(0, 50));
    $effect(() => {
        if (active >= shown.length) active = Math.max(0, shown.length - 1);
    });

    function choose(option: PickerOption | undefined): void {
        if (option === undefined || busy) return;
        onChoose(option.account, option.kind === "create");
    }

    function onkeydown(event: KeyboardEvent): void {
        if (event.key === "ArrowDown") {
            active = Math.min(shown.length - 1, active + 1);
            event.preventDefault();
        } else if (event.key === "ArrowUp") {
            active = Math.max(0, active - 1);
            event.preventDefault();
        } else if (event.key === "Enter") {
            choose(shown[active]);
            event.preventDefault();
        }
    }
</script>

<div class="modal modal-open" role="presentation">
    <div
        class="modal-box flex max-h-[80vh] flex-col"
        role="dialog"
        aria-modal="true"
        aria-labelledby="{uid}-title"
        data-testid="time-picker"
        use:dismissible={{onDismiss: onCancel, trap: true, initial: "input"}}
    >
        <h2 id="{uid}-title" class="text-lg font-semibold">{title}</h2>
        <input
            class="input mt-3 w-full"
            type="text"
            placeholder="Search accounts"
            autocomplete="off"
            spellcheck="false"
            role="combobox"
            aria-expanded="true"
            aria-controls="{uid}-list"
            aria-activedescendant={shown.length > 0 ? `${uid}-opt-${active}` : undefined}
            bind:value={query}
            {onkeydown}
            oninput={() => (active = 0)}
            data-testid="time-picker-input"
        />
        <ul id="{uid}-list" class="menu mt-2 min-h-0 w-full flex-1 flex-nowrap overflow-y-auto" role="listbox" aria-label="Accounts">
            {#if names === null}
                <li class="px-3 py-2 text-sm text-base-content/60">Loading accounts…</li>
            {:else if shown.length === 0}
                <li class="px-3 py-2 text-sm text-base-content/60">No account matches. Type a new name to create it.</li>
            {/if}
            {#each shown as option, i (`${option.kind}:${option.account}`)}
                <li role="option" id="{uid}-opt-{i}" aria-selected={i === active}>
                    <button
                        type="button"
                        tabindex="-1"
                        class={i === active ? "menu-active" : ""}
                        onclick={() => choose(option)}
                        onmouseenter={() => (active = i)}
                        data-testid={option.kind === "create" ? "time-picker-create" : "time-picker-option"}
                    >
                        {#if option.kind === "create"}
                            <span>Create account “{option.account}”…</span>
                        {:else}
                            <span class="truncate">{accountLabel(option.account)}</span>
                            {#if option.account === current}<span class="ml-auto text-xs text-base-content/60">current</span>
                            {:else if recents.includes(option.account) && query.trim() === ""}<span class="ml-auto text-xs text-base-content/60">recent</span
                                >{/if}
                        {/if}
                    </button>
                </li>
            {/each}
        </ul>
        {#if error !== null}
            <div class="mt-3 alert text-sm alert-warning" role="alert" data-testid="time-picker-error">{error}</div>
        {/if}
        <div class="modal-action mt-3">
            {#if onClear !== undefined}
                <button type="button" class="btn mr-auto btn-ghost btn-sm" onclick={onClear} disabled={busy} data-testid="time-picker-clear">Clear pin</button>
            {/if}
            <button type="button" class="btn btn-ghost btn-sm" onclick={onCancel} disabled={busy}>Cancel</button>
        </div>
    </div>
</div>
