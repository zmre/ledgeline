<!-- A list of words (Wording, SETTINGS-PLAN §1) as chips: the first is the
     word the app writes, the rest are also recognized. Type a word and press
     Enter (or a comma) to add it; each chip's × removes it; Backspace in the
     empty field removes the last. -->
<script lang="ts">
    let {
        id,
        value,
        disabled = false,
        onChange,
    }: {
        id: string;
        value: string[];
        disabled?: boolean;
        onChange: (value: string[]) => void;
    } = $props();

    let typed = $state("");

    function add(): void {
        const word = typed.trim();
        typed = "";
        if (word === "" || value.some((w) => w.toLowerCase() === word.toLowerCase())) return;
        onChange([...value, word]);
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Enter" || event.key === ",") {
            event.preventDefault();
            add();
        } else if (event.key === "Backspace" && typed === "" && value.length > 0) {
            onChange(value.slice(0, -1));
        }
    }
</script>

<div class="flex flex-wrap items-center gap-1 rounded-field border border-base-300 bg-base-100 p-1.5" data-testid="word-list">
    {#each value as word, at (word)}
        <span class={["badge gap-1", at === 0 ? "badge-primary" : "badge-ghost"]} title={at === 0 ? "The word the app writes" : undefined}>
            {word}
            <button
                type="button"
                class="btn btn-circle btn-ghost btn-xs"
                aria-label="Remove {word}"
                {disabled}
                onclick={() => onChange(value.filter((_, i) => i !== at))}>×</button
            >
        </span>
    {/each}
    <input
        {id}
        type="text"
        class="input min-w-24 flex-1 border-transparent shadow-none input-xs"
        placeholder="Add a word"
        bind:value={typed}
        {disabled}
        onkeydown={onKeydown}
        onblur={add}
        data-testid="word-list-input"
    />
</div>
