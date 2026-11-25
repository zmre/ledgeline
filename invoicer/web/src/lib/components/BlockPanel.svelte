<!-- Which of the base's blocks a template overrides, and "Override block…"
     (invoicer PLAN 4d, Q1's promise: change one block, or just `theme`).
     The overridden ones are buttons that put the caret in that block; the
     rest are offered in a list with what each is for, and Override inserts
     `{% block name %}{{ super() }}{% endblock %}` (the base's own content
     kept, ready for more). A block the base doesn't have is flagged: Tera
     ignores it. A template that extends no base (a whole page) says so. -->
<script lang="ts">
    import type {BaseBlock} from "#lib/templates.js";

    let {
        blocks,
        defined,
        extendsBase,
        readonly = false,
        onOverride,
        onShow,
    }: {
        /** The base's blocks, in page order. */
        blocks: BaseBlock[];
        /** The blocks the text defines now. */
        defined: string[];
        /** Whether the text extends a base at all. */
        extendsBase: boolean;
        readonly?: boolean;
        onOverride: (name: string) => void;
        onShow: (name: string) => void;
    } = $props();

    const uid = $props.id();
    const known = $derived(new Set(blocks.map((b) => b.name)));
    const overridden = $derived(defined.filter((name) => known.has(name)));
    const unknown = $derived(defined.filter((name) => !known.has(name)));
    const offered = $derived(blocks.filter((b) => !defined.includes(b.name)));

    let picked = $state("");
    const pickedBlock = $derived(offered.find((b) => b.name === picked) ?? offered[0] ?? null);

    function override(): void {
        if (pickedBlock === null) return;
        onOverride(pickedBlock.name);
        picked = "";
    }
</script>

<section class="flex flex-col gap-2 rounded-box border border-base-300 p-3" aria-labelledby="{uid}-title" data-testid="block-panel">
    <h3 id="{uid}-title" class="text-sm font-semibold">Blocks</h3>
    {#if !extendsBase}
        <p class="text-sm text-base-content/70" data-testid="block-panel-whole">
            A whole page: it doesn't build on the invoice base, so there are no blocks to override.
        </p>
    {:else}
        <div class="flex flex-wrap items-center gap-1.5 text-sm">
            <span class="text-base-content/70">Changes from the base:</span>
            {#if overridden.length === 0}
                <span class="text-base-content/70" data-testid="block-panel-none">none yet, so it looks as the base does.</span>
            {/if}
            {#each overridden as name (name)}
                <button
                    type="button"
                    class="badge cursor-pointer badge-outline badge-primary"
                    title="Go to the {name} block"
                    onclick={() => onShow(name)}
                    data-testid="block-overridden">{name}</button
                >
            {/each}
            {#each unknown as name (name)}
                <button
                    type="button"
                    class="badge cursor-pointer badge-outline badge-warning"
                    title="The base has no {name} block, so it changes nothing"
                    onclick={() => onShow(name)}
                    data-testid="block-unknown">{name} (not in the base)</button
                >
            {/each}
        </div>
        {#if !readonly && offered.length > 0}
            <div class="flex flex-wrap items-end gap-2">
                <label class="flex min-w-0 flex-col gap-1 text-sm">
                    <span class="font-medium">Override a block</span>
                    <select class="select max-w-full select-sm" bind:value={picked} data-testid="block-choice">
                        {#each offered as block (block.name)}
                            <option value={block.name}>{block.name}</option>
                        {/each}
                    </select>
                </label>
                <button type="button" class="btn btn-outline btn-sm" onclick={override} data-testid="block-override">Override block</button>
            </div>
            {#if pickedBlock !== null && pickedBlock.about !== ""}
                <p class="text-xs text-base-content/70" data-testid="block-about">{pickedBlock.about}</p>
            {/if}
        {/if}
    {/if}
</section>
