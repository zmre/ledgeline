<script lang="ts">
    // The scenario `<option>`s, grouped, shared by the Projections tab's
    // picker and Settings → Projections (plan 33 D22), so the two cannot
    // label a file differently.
    //
    // `projection-*.journal` files are the ones Ledgeline wrote and come first,
    // under their own heading; everything else is "Other journals". Only the
    // first group has a `; projection:` name. The default scenario (D21) reads
    // "<name> (default)".
    //
    // Options only: the caller owns the `<select>` and whatever leading option
    // it needs ("Seeded from your journal", "None — …").

    import type {ProjectionFile} from "../types";

    interface Props {
        files: readonly ProjectionFile[];
        /** The effective default's id, or null. */
        defaultId: string | null;
    }

    const {files, defaultId}: Props = $props();

    const projections = $derived(files.filter((file) => file.isProjection));
    const others = $derived(files.filter((file) => !file.isProjection));

    /** The picker's label for one file: its `; projection:` name, else its filename label. */
    const labelOf = (file: ProjectionFile): string => {
        const label = file.name ?? file.label;
        return file.id === defaultId ? `${label} (default)` : label;
    };
</script>

{#if projections.length > 0}
    <optgroup label="Projections">
        {#each projections as file (file.id)}
            <option value={file.id}>{labelOf(file)}</option>
        {/each}
    </optgroup>
{/if}
{#if others.length > 0}
    <optgroup label="Other journals">
        {#each others as file (file.id)}
            <option value={file.id}>{labelOf(file)}</option>
        {/each}
    </optgroup>
{/if}
