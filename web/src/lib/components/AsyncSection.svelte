<!-- The error / loading / data tri-state for a surface backed by one async
     store — the ONE place that chain is written.

     It was written four times, verbatim, and FE-5 was a defect in all four at
     once: the "render the data" branch was tested BEFORE the error branch, and
     the error branch additionally required the payload to be null. Once
     anything had loaded, the error branch was unreachable. A refetch that 500'd
     left the PREVIOUS answer on screen — December's balance sheet under a
     control reading June — with nothing to say it had failed.

     The order below is the fix, and now it is structural: a surface cannot get
     it wrong without editing this file. `dataView` (lib/stores/loadState.ts)
     still decides WHICH branch; this only renders it. Error outranks stale data
     deliberately — a surface that cannot honour the current request must say
     so, not keep quietly serving the previous one.

     `value` is passed to the data snippet rather than read from a store so the
     caller gets non-null narrowing without re-testing it. -->
<script lang="ts" generics="T">
    import type {Snippet} from "svelte";
    import {NativeApiUnavailableError} from "$lib/api/native";
    import type {DataView} from "$lib/stores/loadState";

    let {
        view,
        value,
        error,
        testid,
        label,
        loadingLabel,
        onRetry,
        children,
    }: {
        /** Which branch to render — from `dataView`, never hand-rolled. */
        view: DataView;
        /** The held payload, handed to `children` once it is worth showing. */
        value: T | null;
        error: Error | null;
        /** `data-testid` for the error alert; the e2e suite selects on it. */
        testid: string;
        /** Names the thing that failed: "Couldn't load {label}: …". */
        label: string;
        /** Accessible name for the loading spinner, e.g. "Loading reports". */
        loadingLabel: string;
        onRetry: () => void;
        children: Snippet<[T]>;
    } = $props();

    // "The engine isn't running" already reads as a full sentence and names its
    // own remedy, so it is shown bare and without a Retry that cannot help.
    const nativeUnavailable = $derived(error instanceof NativeApiUnavailableError);
    const message = $derived(nativeUnavailable ? (error?.message ?? "") : `Couldn't load ${label}: ${error?.message ?? "unknown error"}`);
</script>

{#if view === "error"}
    <div class="alert alert-error rounded-box flex-col items-start gap-2 px-3 py-3 text-sm" role="alert" data-testid={testid}>
        <span>{message}</span>
        {#if !nativeUnavailable}
            <button type="button" class="btn btn-sm" onclick={onRetry}>Retry</button>
        {/if}
    </div>
{:else if view === "data" && value !== null}
    {@render children(value)}
{:else}
    <div class="flex items-center justify-center py-24" aria-label={loadingLabel}>
        <span class="loading loading-spinner loading-lg"></span>
    </div>
{/if}
