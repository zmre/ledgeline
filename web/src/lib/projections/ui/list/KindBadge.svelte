<!-- A kind badge on an entry's card: small caps, tinted, and always TEXT —
     the colour repeats the word, it never replaces it (plan 31 C12).

     The tints are the app's own: income and expense take the cash-flow
     chart's in/out colours (`--chart-in`, `--chart-out`), so a card reads in
     the same colours as the charts under the list. -->
<script lang="ts">
    import type {Badge} from "../../entries";

    let {kind}: {kind: Badge | "off" | "estimated" | "section" | "note"} = $props();

    const LABEL: Record<typeof kind, string> = {
        income: "income",
        expense: "expense",
        asset: "asset",
        oneoff: "one-off",
        auto: "auto",
        off: "off",
        estimated: "estimated",
        section: "section",
        note: "note",
    };

    // Spelled out, not interpolated, so Tailwind sees every class.
    const TINT: Record<typeof kind, string> = {
        income: "bg-(--chart-in)/12 text-(--chart-in)",
        expense: "bg-(--chart-out)/14 text-(--chart-out)",
        asset: "bg-info/12 text-info",
        oneoff: "bg-accent/12 text-accent",
        auto: "bg-secondary/14 text-secondary",
        off: "bg-base-content/10 text-base-content/70",
        estimated: "bg-warning/10 text-warning/90",
        section: "bg-base-content/8 text-base-content/55",
        note: "bg-base-content/8 text-base-content/55",
    };
</script>

<span
    class="inline-flex h-[1.125rem] shrink-0 items-center rounded-[0.3rem] px-1.5 font-sans text-[0.625rem] leading-none font-semibold tracking-[0.08em] uppercase {TINT[
        kind
    ]}"
    data-badge={kind}
    >{#if kind === "off"}<span class="sr-only">turned&nbsp;</span>{/if}{LABEL[kind]}</span
>
