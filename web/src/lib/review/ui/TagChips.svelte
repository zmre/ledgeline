<script lang="ts">
    // The card's header tags as chips, plus one input that completes a tag NAME
    // and then, after `:`, its VALUE (plan 32 D15). Enter or `,` adds the tag
    // (replacing one of the same name, as `setTxnTag` would); Backspace in an
    // empty input takes the last chip off. Only what `parseTag` accepts — what
    // the engine can write surgically — can become a chip.
    import {tick} from "svelte";
    import {TYPING_ATTRIBUTE} from "#lib/keys/target.js";
    import {popupPosition, portal} from "#lib/ui/anchoredPopup.js";
    import {parseTag, tagCompletions, type TagCompletion} from "../complete";
    import type {TagUse} from "../types";
    import ReviewIcon from "./ReviewIcon.svelte";

    let {tags = $bindable([]), vocabulary, disabled = false}: {tags?: [string, string][]; vocabulary: readonly TagUse[]; disabled?: boolean} = $props();

    const VISIBLE_ROWS = 8;
    const ROW_HEIGHT = 30;

    let text = $state("");
    let open = $state(false);
    let highlighted = $state(0);
    let invalid = $state(false);
    let field = $state<HTMLInputElement | null>(null);
    let placement = $state({top: 0, left: 0, width: 0, maxHeight: 0, below: true});
    let choosing = false;

    const taken = $derived(new Set(tags.map(([name]) => name)));
    const matches = $derived<TagCompletion[]>(open ? tagCompletions(text, vocabulary, taken) : []);
    const uid = $props.id();
    const listId = `tag-list-${uid}`;

    function reposition(): void {
        if (field !== null)
            placement = popupPosition(field.getBoundingClientRect(), {width: window.innerWidth, height: window.innerHeight}, VISIBLE_ROWS * ROW_HEIGHT);
    }

    async function show(): Promise<void> {
        open = true;
        highlighted = 0;
        await tick();
        reposition();
    }

    function add(): boolean {
        if (text.trim() === "") return false;
        const tag = parseTag(text);
        if (tag === null) {
            invalid = true;
            return true;
        }
        tags = [...tags.filter(([name]) => name !== tag[0]), tag];
        text = "";
        invalid = false;
        open = false;
        return true;
    }

    function remove(name: string): void {
        tags = tags.filter(([candidate]) => candidate !== name);
        field?.focus();
    }

    function choose(completion: TagCompletion): void {
        text = completion.insert;
        field?.focus();
        if (completion.kind === "name") void show();
        else add();
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.isComposing) return;
        const live = open && matches.length > 0;
        if (event.key === "Escape") {
            if (open) {
                event.preventDefault();
                open = false;
            }
            return;
        }
        if (event.key === "ArrowDown") {
            event.preventDefault();
            if (live) highlighted = (highlighted + 1) % matches.length;
            else void show();
            return;
        }
        if (event.key === "ArrowUp" && live) {
            event.preventDefault();
            highlighted = (highlighted - 1 + matches.length) % matches.length;
            return;
        }
        if ((event.key === "Tab" && !event.shiftKey && live) || (event.key === "Enter" && live && !(event.metaKey || event.ctrlKey))) {
            event.preventDefault();
            const completion = matches[highlighted];
            if (completion !== undefined) choose(completion);
            return;
        }
        // Plain Enter or a comma adds what is typed. ⌘↵ is left alone: it saves.
        if ((event.key === "Enter" && !(event.metaKey || event.ctrlKey)) || event.key === ",") {
            if (add()) event.preventDefault();
            return;
        }
        if (event.key === "Backspace" && text === "" && tags.length > 0) {
            event.preventDefault();
            tags = tags.slice(0, -1);
        }
    }
</script>

<div class="flex min-h-8 flex-wrap items-center gap-1.5" {...{[TYPING_ATTRIBUTE]: ""}}>
    {#each tags as [name, value] (name)}
        <span class="badge gap-1 border-base-content/15 bg-base-200 font-mono badge-sm" data-testid="review-tag">
            <span class="text-base-content/60">{name}:</span><span>{value}</span>
            <button
                type="button"
                class="-mr-1 rounded-full p-0.5 hover:bg-base-content/10"
                aria-label="Remove tag {name}"
                onclick={() => remove(name)}
                {disabled}
            >
                <ReviewIcon name="x" class="h-3 w-3" />
            </button>
        </span>
    {/each}
    <input
        bind:this={field}
        type="text"
        class="input w-36 grow font-mono input-xs {invalid ? 'input-error' : ''}"
        bind:value={text}
        placeholder={tags.length === 0 ? "name:value" : "+ tag"}
        {disabled}
        role="combobox"
        aria-label="Add a tag"
        aria-invalid={invalid}
        aria-expanded={open}
        aria-controls={listId}
        aria-autocomplete="list"
        autocomplete="off"
        spellcheck="false"
        oninput={() => {
            invalid = false;
            void show();
        }}
        onkeydown={onKeydown}
        onblur={() => !choosing && (open = false)}
    />
</div>
{#if invalid}
    <p class="mt-1 text-xs text-error" role="alert">A tag is <code>name:value</code>; the name has no spaces, and neither has a comma.</p>
{/if}

{#if open && matches.length > 0}
    <ul
        use:portal
        id={listId}
        role="listbox"
        aria-label="Tag suggestions"
        class="menu fixed z-[1001] flex-nowrap overflow-y-auto rounded-box border border-base-300 bg-base-200 p-1 shadow-lg"
        style="top: {placement.top}px; left: {placement.left}px; width: {Math.max(placement.width, 200)}px; max-height: {placement.maxHeight}px"
        {...{[TYPING_ATTRIBUTE]: ""}}
    >
        {#each matches as match, at (match.insert)}
            <li>
                <button
                    type="button"
                    role="option"
                    aria-selected={at === highlighted}
                    class="flex w-full items-baseline justify-between gap-3 px-2 py-1 text-left font-mono text-sm {at === highlighted ? 'bg-primary/25' : ''}"
                    tabindex="-1"
                    onpointerdown={() => (choosing = true)}
                    onclick={() => {
                        choosing = false;
                        choose(match);
                    }}
                >
                    <span class="truncate">{match.kind === "name" ? `${match.label}:` : match.label}</span>
                    <span class="shrink-0 text-xs text-base-content/50 tabular-nums">{match.count}×</span>
                </button>
            </li>
        {/each}
    </ul>
{/if}

<svelte:window onresize={() => open && reposition()} />
