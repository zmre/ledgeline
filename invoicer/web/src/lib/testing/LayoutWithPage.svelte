<script lang="ts">
    // The root layout's app keys with a page's keys under it, mounted as the
    // app mounts them: the layout (this component) registers `appLayer`, and
    // its child, the page, registers its own layer. On a first load Svelte
    // runs the child's effect first, so the page registers BEFORE the layout,
    // which is the order that used to let the app's `n` win.
    import {untrack} from "svelte";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import type {Layer} from "#ui/keys/types.js";
    import KeysProbe from "#ui/testing/fixtures/KeysProbe.svelte";
    import {appLayer} from "#lib/keys/appKeys.js";

    let {pageLayer}: {pageLayer: () => Layer} = $props();

    registerKeys(appLayer());
    const page = untrack(() => pageLayer);
</script>

<KeysProbe layerOf={page} />
