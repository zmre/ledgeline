<!-- Until Phase 5's dashboard, the app opens on the invoice list. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";

    onMount(() => {
        void goto(resolve("/invoices"), {replaceState: true});
    });
</script>
