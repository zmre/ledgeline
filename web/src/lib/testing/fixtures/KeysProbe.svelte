<script lang="ts">
    // Mounts a keymap layer so a test can exercise register/unregister through a
    // real component lifecycle. `registerKeys` declares an `$effect`, so it can
    // only be called during component init — there is no way to test its cleanup
    // without a component to unmount.
    //
    // The prop is a FACTORY, invoked inside `untrack`, because reading a
    // `$props()` value directly at init trips `state_referenced_locally` and
    // this repo carries no `svelte-ignore` comments. Reading it inside a closure
    // is what the warning asks for, and "read once at init, never react to it
    // again" is exactly `registerKeys`'s documented contract — so saying so with
    // `untrack` is more honest than suppressing the warning would be.
    import {untrack} from "svelte";
    import {registerKeys} from "$lib/keys/keymap.svelte";
    import type {Layer} from "$lib/keys/types";

    let {layerOf}: {layerOf: () => Layer} = $props();

    registerKeys(untrack(() => layerOf()));
</script>
