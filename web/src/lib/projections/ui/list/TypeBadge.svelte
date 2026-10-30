<!-- An account-TYPE badge on one posting line: what kind of account the money
     moves through, read from its declared (or inferred) type — never from
     which posting comes first (Patrick's 2026-10-29 feedback).

     Always text; the tint repeats the word. Income and expense take the
     cash-flow chart's in/out colours, so a card reads in the charts' colours.
     An account with no type says so with a neutral "?" whose title explains
     how to give it one; a blank account draws nothing. `dim` is the greyed
     default counter-account's badge, dimmed like its text. -->
<script lang="ts">
    import type {AccountClasses} from "#lib/domain/accountClass.js";
    import {typeBadgeOf, type TypeBadge} from "../../entries";

    let {account, classes, dim = false}: {account: string; classes: AccountClasses; dim?: boolean} = $props();

    const type = $derived(typeBadgeOf(account, classes));

    const LABEL: Record<TypeBadge, string> = {
        income: "income",
        expense: "expense",
        asset: "asset",
        cash: "cash",
        liability: "liability",
        equity: "equity",
        gain: "gain",
        conversion: "conversion",
    };

    // Spelled out, not interpolated, so Tailwind sees every class.
    const TINT: Record<TypeBadge, string> = {
        income: "bg-(--chart-in)/12 text-(--chart-in)",
        expense: "bg-(--chart-out)/14 text-(--chart-out)",
        asset: "bg-info/12 text-info",
        cash: "bg-primary/16 text-primary",
        liability: "bg-warning/12 text-warning",
        equity: "bg-secondary/14 text-secondary",
        gain: "bg-base-content/10 text-base-content/70",
        conversion: "bg-base-content/10 text-base-content/70",
    };

    const BASE =
        "inline-flex h-4 max-w-full shrink-0 items-center self-center justify-self-start rounded-[0.3rem] px-1.5 font-sans text-[0.5625rem] leading-none font-semibold tracking-[0.07em] uppercase";
</script>

{#if account.trim() === ""}
    <span aria-hidden="true"></span>
{:else if type === null}
    <span
        class={[BASE, "bg-base-content/6 text-base-content/45", dim && "opacity-60"]}
        title="{account} has no account type. Declare one in the journal or this file (account {account}  ; type: Asset, Liability, Equity, Revenue or Expense)."
        data-type-badge="none"><span aria-hidden="true">?</span><span class="sr-only">no account type</span></span
    >
{:else}
    <span class={[BASE, TINT[type], dim && "opacity-60"]} data-type-badge={type}>{LABEL[type]}</span>
{/if}
