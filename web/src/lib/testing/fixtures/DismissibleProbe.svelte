<script lang="ts">
    // A minimal overlay for exercising the `dismissible` action: two focusable
    // children so Tab wrapping has something to wrap between.
    import {untrack} from "svelte";
    import {dismissible, type DismissibleOptions} from "$lib/keys/dismissible";

    let {optionsOf}: {optionsOf: () => DismissibleOptions} = $props();

    // Read once (see KeysProbe for why the factory + untrack shape).
    const options = untrack(() => optionsOf());
</script>

<div use:dismissible={options} data-testid="overlay">
    <button type="button">first</button>
    <button type="button">last</button>
</div>
