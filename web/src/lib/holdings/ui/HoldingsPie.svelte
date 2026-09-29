<!-- Holdings pie (WP-10), divisible by holding or by category.
     "By holding" (the default) is one slice per symbol by toNumber(marketValue),
     unpriced holdings excluded (the inline warning covers them), tail folded
     into one "(other)" bucket — rendered from the report alone.
     The category views (asset class, sector, industry, security type,
     category, risk) split each holding's value by its classification from
     GET /api/holdings/profiles (commodity tags over Yahoo Finance), fetched
     the first time one is chosen — see $lib/holdings/categories.ts.
     - colors: the theme's categorical chart tokens via `chartColors` — every
       slot the theme defines, in fixed order, plus muted gray for the folded tail, and that gray
       at partial opacity for "(unclassified)": value nothing classifies is
       shown (it is real money) but never dressed as a category, and never
       mistaken for the fold. Asset classes and security types keep a fixed
       slot, so Equity is one colour in every scope.
       $lib/format/palette documents the validator run and why ORDER matters.
       Secondary encoding, which the skill requires at this CVD separation, is
       the always-visible legend (label + % share, identity never color-alone),
       the pad-angle gaps between slices, and the tooltips.
     - a slice past the palette's last slot never gets a generated hue: it
       folds into "(other)" (dataviz non-negotiable), which is why the
       named-slice cap is the palette's slot count. -->
<script lang="ts">
    import {PieChart, Tooltip} from "layerchart";
    import ChartLegend from "$lib/components/ChartLegend.svelte";
    import type {Dec} from "$lib/domain/money";
    import {chartColors} from "$lib/format/chartColors.svelte";
    import {unknownColor} from "$lib/format/palette";
    import {availableDimensions, categorySlices, type CategorySlice} from "$lib/holdings/categories";
    import {CATEGORY_DIMENSIONS, type CategoryDimension, type PieDimension} from "$lib/holdings/profileTypes";
    import type {Holding} from "$lib/holdings/types";
    import {holdingsProfiles} from "$lib/stores/holdingsProfiles.svelte";
    import {settings} from "$lib/stores/settings.svelte";
    import {pieSlices, PIE_OTHER, type PieSlice} from "./view";

    let {holdings, format}: {holdings: Holding[]; format: (v: Dec) => string} = $props();

    const DIMENSION_LABELS: Record<PieDimension, string> = {
        holding: "By holding",
        assetClass: "Asset class",
        sector: "Sector",
        industry: "Industry",
        securityType: "Security type",
        category: "Category",
        risk: "Risk (Morningstar)",
    };

    /**
     * The tag that classifies a commodity along each dimension — named in the
     * empty state. Mirrors `Dimension::tag()` in
     * crates/ledgeline-server/src/commodity_profile.rs, which is what reads the
     * tag; the two cannot share code, so change both together.
     */
    const DIMENSION_TAGS: Record<CategoryDimension, string> = {
        assetClass: "assetclass",
        sector: "sector",
        industry: "industry",
        securityType: "type",
        category: "category",
        risk: "risk",
    };

    const dimension = $derived(settings.holdingsPieDimension);
    const category = $derived(dimension === "holding" ? null : dimension);

    // Fetch classifications only once a category view is showing, and once per
    // report (the store dedupes on the rows' identity).
    $effect(() => {
        const url = settings.serverUrl;
        const nonce = settings.serverNonce;
        if (category !== null && url !== null) void holdingsProfiles.ensure(url, nonce, holdings);
    });

    const profiles = $derived(holdingsProfiles.value);
    const loading = $derived(category !== null && holdingsProfiles.status === "loading");
    const failed = $derived(category !== null && holdingsProfiles.status === "error");

    // Before any answer there is no honest way to know which views have data,
    // so every one is offered; afterwards only those some shown holding has
    // data for — plus whichever is selected, so the control never shows a
    // value it does not list.
    const offered = $derived.by((): PieDimension[] => {
        const known = profiles === null ? CATEGORY_DIMENSIONS : availableDimensions(holdings, profiles);
        return ["holding", ...CATEGORY_DIMENSIONS.filter((d) => known.includes(d) || d === dimension)];
    });

    const slices = $derived(category === null || profiles === null ? [] : categorySlices(holdings, profiles, category, chartColors.categorical.length));
    const onlyUnclassified = $derived(slices.length > 0 && slices.every((s) => s.kind === "unclassified"));

    const categoryColor = (slice: CategorySlice): string => {
        if (slice.kind === "unclassified") return unknownColor(chartColors.current);
        if (slice.kind === "other" || slice.slot === null) return chartColors.other;
        return chartColors.colorAt(slice.slot);
    };

    const yahooHint = $derived.by((): string | null => {
        if (category === null || profiles === null) return null;
        if (profiles.yahoo === "unavailable") return "Couldn't reach Yahoo — showing tag data only";
        if (profiles.yahoo === "partial") return "Yahoo didn't answer for some holdings — they show tag data only";
        return null;
    });

    const retry = (): void => {
        if (settings.serverUrl !== null) void holdingsProfiles.reload(settings.serverUrl, settings.serverNonce, holdings);
    };

    const describe = (slice: CategorySlice): string => {
        if (slice.kind === "unclassified") return "No commodity tag or Yahoo data for this view";
        const n = slice.holdings;
        return `${n} holding${n === 1 ? "" : "s"}`;
    };

    /** One drawn slice, whichever way the pie is divided: its wedge, tooltip and legend entry. */
    interface Wedge {
        key: string;
        /** The legend's name for it: a symbol, or a category. */
        label: string;
        /** The tooltip's name for it: the holding's full name, or the category. */
        name: string;
        color: string;
        value: number;
        formatted: string;
        /** A second tooltip line, when there is more to say. */
        detail: string | null;
        /** The muted figure after the legend label. */
        extra: string;
        title: string;
        italic: boolean;
    }

    const share = (percent: number): string => `${percent.toFixed(1)}%`;

    const holdingWedge = (slice: PieSlice, i: number): Wedge => ({
        key: slice.symbol,
        label: slice.symbol,
        name: slice.name,
        color: slice.symbol === PIE_OTHER ? chartColors.other : chartColors.colorAt(i),
        value: slice.value,
        formatted: slice.formatted,
        detail: null,
        extra: share(slice.share),
        title: `${slice.name} — ${slice.formatted}`,
        italic: false,
    });

    const categoryWedge = (slice: CategorySlice): Wedge => {
        const formatted = format(slice.amount);
        return {
            key: slice.key,
            label: slice.label,
            name: slice.label,
            color: categoryColor(slice),
            value: slice.value,
            formatted,
            detail: `${share(slice.share)} · ${describe(slice)}`,
            extra: `${share(slice.share)} · ${formatted}`,
            title: `${slice.label} — ${formatted} (${describe(slice)})`,
            italic: slice.kind === "unclassified",
        };
    };

    const wedges = $derived(category === null ? pieSlices(holdings, format, chartColors.categorical.length).map(holdingWedge) : slices.map(categoryWedge));
</script>

<div class="flex flex-col gap-1" data-testid="holdings-pie-panel">
    <div class="flex flex-wrap items-center justify-between gap-2">
        <label class="flex items-center gap-2 text-xs text-base-content/60">
            <span class="sr-only sm:not-sr-only">Divide by</span>
            <select
                class="select w-auto select-xs"
                aria-label="Divide the pie by"
                data-testid="holdings-pie-dimension"
                value={dimension}
                onchange={(event) => (settings.holdingsPieDimension = event.currentTarget.value as PieDimension)}
            >
                {#each offered as option (option)}
                    <option value={option}>{DIMENSION_LABELS[option]}</option>
                {/each}
            </select>
        </label>
        {#if loading}
            <span class="flex items-center gap-1 text-xs text-base-content/60" role="status" data-testid="holdings-pie-loading">
                <span class="loading loading-xs loading-spinner"></span>
                Classifying holdings…
            </span>
        {:else if failed && profiles !== null}
            <!-- An older answer is still on screen; say it could not be refreshed rather than pass it off as current. -->
            <span class="flex items-center gap-1 text-xs text-base-content/60" data-testid="holdings-pie-stale">
                Couldn't refresh classifications.
                <button type="button" class="link" onclick={retry}>Retry</button>
            </span>
        {:else if yahooHint !== null}
            <span class="text-xs text-base-content/60" data-testid="holdings-pie-yahoo-hint">{yahooHint}</span>
        {/if}
    </div>

    {#if category !== null && profiles === null}
        <!-- No answer yet: hold the pie's own height so nothing below jumps when it lands. -->
        <div class="flex h-56 w-full flex-col items-center justify-center gap-2 text-sm text-base-content/60 sm:h-64" data-testid="holdings-pie-pending">
            {#if failed}
                <p data-testid="holdings-pie-error">Couldn't load classifications.</p>
                <button type="button" class="btn btn-ghost btn-xs" onclick={retry}>Retry</button>
            {:else}
                <span class="loading loading-md loading-spinner" aria-hidden="true"></span>
            {/if}
        </div>
    {:else if wedges.length === 0}
        <p class="py-10 text-center text-sm text-base-content/60">No priced holdings to chart.</p>
    {:else if category !== null && onlyUnclassified}
        <div
            class="flex h-56 w-full flex-col items-center justify-center gap-1 px-4 text-center text-sm text-base-content/60 sm:h-64"
            data-testid="holdings-pie-unclassified"
        >
            <p>No holding here has {DIMENSION_LABELS[category].toLowerCase()} data.</p>
            <p class="text-xs">
                Add it to a commodity directive, e.g. <code class="text-base-content/80">commodity VTI&nbsp;&nbsp;; {DIMENSION_TAGS[category]}: …</code>
            </p>
        </div>
    {:else}
        <!-- A refetch keeps the previous pie at reduced opacity rather than flashing a spinner (dataviz: no skeleton on refetch). -->
        <div class={["h-56 w-full transition-opacity sm:h-64", loading && "opacity-50"]} data-testid="holdings-pie">
            <PieChart data={wedges} key="key" label="label" value={(d) => d.value} cRange={wedges.map((d) => d.color)} padAngle={0.02}>
                {#snippet tooltip()}
                    <Tooltip.Root>
                        {#snippet children({data})}
                            {@const d = data as Wedge}
                            <div class="flex flex-col gap-0.5 text-xs">
                                <div class="flex items-center gap-2">
                                    <span class="inline-block h-2 w-2 rounded-full" style="background:{d.color}"></span>
                                    <span class="text-base-content/70">{d.name}</span>
                                    <span class="font-semibold">{d.formatted}</span>
                                </div>
                                {#if d.detail !== null}<span class="text-base-content/60">{d.detail}</span>{/if}
                            </div>
                        {/snippet}
                    </Tooltip.Root>
                {/snippet}
            </PieChart>
        </div>
        <!-- always-visible legend: name + % share (+ value by category); identity is never color-alone -->
        <ChartLegend entries={wedges} class="mt-1" testid="holdings-pie-legend" />
    {/if}
</div>
