<script lang="ts">
    // A `PRIORITY.overlay`, `modal: true` key layer for an overlay that is
    // always mounted (the transaction popup): render it inside the overlay's
    // `{#if open}`, and it exists exactly while the overlay is up. A modal layer
    // blinds everything below it, so the journal's j/k/x cannot fire behind the
    // popup while focus sits on one of its buttons (plan 32 D16).
    //
    // `?` is re-bound here because the modal layer hides the global one.
    import {untrack} from "svelte";
    import {keymap, registerKeys} from "./keymap.svelte";
    import {PRIORITY, type Binding} from "./types";

    let {id, bindings = []}: {id: string; bindings?: Binding[]} = $props();

    // A layer is registered once, for the component's lifetime: its inputs are read once.
    const layer = untrack(() => ({id, bindings}));

    registerKeys({
        id: layer.id,
        priority: PRIORITY.overlay,
        modal: true,
        bindings: [...layer.bindings, {keys: "?", label: "Show keyboard shortcuts", group: "Global", run: () => keymap.toggleHelp()}],
    });
</script>
