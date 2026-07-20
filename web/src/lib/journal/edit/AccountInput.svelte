<script lang="ts">
    // Account text field with native autocomplete over the journal's account
    // names (a <datalist> — no bespoke combobox). Used by the popup's posting
    // rows and the inline category editor. Emits value changes via bind:value;
    // Enter commits, Escape cancels (both forwarded to the parent).
    let {
        value = $bindable(""),
        accountNames,
        placeholder = "account",
        autofocus = false,
        size = "sm",
        disabled = false,
        onCommit,
        onCancel,
    }: {
        value?: string;
        accountNames: string[];
        placeholder?: string;
        autofocus?: boolean;
        size?: "sm" | "xs";
        disabled?: boolean;
        onCommit?: () => void;
        onCancel?: () => void;
    } = $props();

    // A stable, unique list id so multiple inputs on one page don't collide.
    const listId = `acct-${Math.random().toString(36).slice(2)}`;

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === "Enter") {
            event.preventDefault();
            onCommit?.();
        } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel?.();
        }
    }

    // Svelte's `autofocus` warning is fine here: focus follows an explicit click.
    function focusOnMount(node: HTMLInputElement): void {
        if (autofocus) node.focus();
    }
</script>

<input
    type="text"
    class="input {size === 'xs' ? 'input-xs' : 'input-sm'} w-full"
    list={listId}
    bind:value
    {placeholder}
    {disabled}
    aria-label="Account"
    autocomplete="off"
    spellcheck="false"
    onkeydown={onKeydown}
    onblur={() => onCommit?.()}
    use:focusOnMount
/>
<datalist id={listId}>
    {#each accountNames as name (name)}
        <option value={name}></option>
    {/each}
</datalist>
