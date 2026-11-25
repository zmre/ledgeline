<!-- Test-only (ProseField.rich.svelte.test.ts): a page holding a prose field's
     text in `form`, as the snippet and invoice pages do, with `reassert`
     re-selecting the same note (a live reload: a new `form`, the same text). -->
<script lang="ts">
    import ProseField from "./ProseField.svelte";

    let {onChange}: {onChange: (value: string) => void} = $props();
    let form = $state({body: ""});

    export function reassert(): void {
        form = {...form};
    }
    export function set(body: string): void {
        form = {body};
    }
</script>

<ProseField
    value={form.body}
    label="Snippet text"
    testid="prose"
    onChange={(value) => {
        form.body = value;
        onChange(value);
    }}
/>
