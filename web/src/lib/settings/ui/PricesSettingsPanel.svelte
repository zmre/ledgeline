<script lang="ts">
    // Settings → Prices (plan 41): automatic stock price updates.
    //
    // The engine does the work (`auto_prices.rs`); this panel only edits its
    // two settings. Both are pinnable, like the theme: "Use for this ledger
    // only" writes them to the ledger's ledgeline.toml, so the user's real
    // books can fetch prices while a sample ledger they demo with does not.
    //
    // The help text matters more than the controls. The updater never chooses
    // a file for a journal's FIRST price, so with the setting on and no prices
    // file nothing at all happens — the text says how to start it.
    import {AUTO_PRICES_WEEKS_RANGE} from "#lib/settings/uiPrefs.js";
    import {settings} from "#lib/stores/settings.svelte.js";

    let pinBusy = $state(false);
    async function pin(pinned: boolean): Promise<void> {
        pinBusy = true;
        try {
            // Both keys together: one setting as far as a person is concerned.
            await settings.pinToLedger("autoPrices", pinned);
            await settings.pinToLedger("autoPricesWeeks", pinned);
        } finally {
            pinBusy = false;
        }
    }

    const pinned = $derived(settings.isPinned("autoPrices"));
</script>

<section class="flex max-w-3xl flex-col gap-4" data-testid="settings-prices">
    <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h2 class="text-base font-semibold">Automatic price updates</h2>
        {#if settings.canPinToLedger}
            <label class="flex cursor-pointer items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    class="checkbox checkbox-sm"
                    checked={pinned}
                    disabled={pinBusy}
                    onchange={(e) => void pin(e.currentTarget.checked)}
                    data-testid="prices-pin"
                />
                Use for this ledger only
            </label>
        {/if}
    </div>

    <label class="flex cursor-pointer items-center gap-3">
        <input
            type="checkbox"
            class="toggle toggle-primary"
            checked={settings.autoPrices}
            onchange={(e) => (settings.autoPrices = e.currentTarget.checked)}
            data-testid="prices-auto"
        />
        <span>Keep stock prices up to date in the background</span>
    </label>

    <label class="flex flex-wrap items-center gap-3">
        <span>Fill in a weekly closing price for the last</span>
        <!-- The number and its unit wrap as one, never "6" on one line and "weeks" on the next. -->
        <span class="flex items-center gap-3">
            <input
                type="number"
                class="input w-20 input-sm"
                min={AUTO_PRICES_WEEKS_RANGE[0]}
                max={AUTO_PRICES_WEEKS_RANGE[1]}
                step="1"
                value={settings.autoPricesWeeks}
                disabled={!settings.autoPrices}
                onchange={(e) => {
                    settings.autoPricesWeeks = e.currentTarget.valueAsNumber;
                    // Show the clamped value, not what was typed.
                    e.currentTarget.value = String(settings.autoPricesWeeks);
                }}
                data-testid="prices-weeks"
            />
            <span>weeks</span>
        </span>
    </label>

    <div class="flex flex-col gap-2 text-sm text-base-content/70">
        <p>
            Shortly after Ledgeline starts, and about once an hour while it stays open, it checks your journal for stocks that are missing the latest closing
            price, or that have no price in one of the past weeks above. Only those are fetched from Yahoo Finance, the same source the
            <span class="font-medium">Update prices</span> button uses. A week counts as covered by any price dated in it. Weeks before you bought a stock are filled
            in too; weeks after you sold all of it are not. Set the weeks to 0 to fetch only the latest close.
        </p>
        <p>
            Nothing happens until your journal has a file holding prices. Click <span class="font-medium">Update prices</span> once on Holdings → Stocks to
            create <code>prices.journal</code>; after that, new prices are added to whichever file holds the most of them, in date order.
        </p>
        {#if pinned}
            <p>This ledger has its own setting, kept in its <code>ledgeline.toml</code>.</p>
        {/if}
    </div>
</section>
