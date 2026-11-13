<!-- File → New (plan 39 Phase 6): the new-ledger wizard.

     Two paths. "Start fresh" asks for a starting chart (personal, business or
     empty), the base currency and fiscal year, shows the accounts for review,
     then a folder and a name, and writes main.journal + accounts.journal.
     "Start from another app" asks which app and which export, writes an
     EMPTY ledger the same way, and hands the file to the Imports screen.

     Everything the engine does is in `wizardStore`; this component only
     renders a step and calls the page's `onDone` once a ledger is open. -->
<script lang="ts">
    import {onMount} from "svelte";
    import {wizard} from "../wizardStore.svelte";
    import {CURRENCY_PRESETS, CUSTOM_CURRENCY, KIND_LABELS, MONTHS, sampleAmount, SOURCE_APPS, sourceFileWarning} from "../model";
    import type {LedgerKind, WizardPath, WizardStep} from "../types";
    import AccountReview from "./AccountReview.svelte";

    let {
        onDone,
        onCancel,
    }: {
        /** A ledger is open: show it, or (after "another app") the import it was made for. */
        onDone: (destination: "journal" | "import") => void;
        /** Leave without creating anything, back to the open journal. */
        onCancel: () => void;
    } = $props();

    onMount(() => {
        void wizard.open();
    });

    const STEP_LABELS: Record<WizardStep, string> = {
        start: "Start",
        kind: "Chart",
        source: "Export",
        details: "Currency",
        accounts: "Accounts",
        create: "Folder",
    };

    const PATHS: {id: WizardPath; title: string; blurb: string}[] = [
        {id: "fresh", title: "Start fresh", blurb: "A new set of books from a starting chart of accounts."},
        {id: "import", title: "Start from another app", blurb: "Quicken, Banktivity, Xero, QuickBooks and others: bring in what you exported."},
    ];
    const KINDS: LedgerKind[] = ["individual", "business", "empty"];

    const desktop = $derived(wizard.status?.desktop ?? true);
    const working = $derived(wizard.busy !== null && wizard.busy !== "template");
    const warning = $derived(
        wizard.source !== null && wizard.sourceFile !== null ? sourceFileWarning(wizard.source, wizard.sourceFile.name, wizard.readableFormats) : null
    );

    async function create(): Promise<void> {
        const outcome = await wizard.create();
        if (!outcome.ok) return;
        if (wizard.path === "import") {
            await wizard.handOffToImport();
            onDone("import");
        } else {
            onDone("journal");
        }
    }

    async function trySample(): Promise<void> {
        const outcome = await wizard.trySample();
        if (outcome.ok) onDone("journal");
    }
</script>

<div class="mx-auto flex w-full max-w-3xl flex-col gap-4 py-4" data-testid="new-ledger-wizard">
    <header class="flex flex-col gap-1">
        <h1 class="text-2xl font-semibold">New ledger</h1>
        <p class="text-sm text-base-content/70">
            Ledgeline keeps your books as plain hledger files in a folder you choose. Nothing is written until the last step.
        </p>
    </header>

    {#if wizard.status !== null && !desktop}
        <div role="alert" class="alert alert-info" data-testid="new-ledger-needs-desktop">
            Creating a ledger needs the Ledgeline desktop app, which can show a folder picker. In a browser, start the server with the journal you want instead.
        </div>
    {/if}

    {#if wizard.steps.length > 1}
        <ul class="steps text-xs">
            {#each wizard.steps as each (each)}
                <li class="step {wizard.steps.indexOf(each) <= wizard.steps.indexOf(wizard.step) ? 'step-primary' : ''}">{STEP_LABELS[each]}</li>
            {/each}
        </ul>
    {/if}

    <section class="card border border-base-300 bg-base-100">
        <div class="card-body gap-4">
            {#if wizard.step === "start"}
                <div class="grid gap-3 sm:grid-cols-2" role="radiogroup" aria-label="How to start">
                    {#each PATHS as option (option.id)}
                        <button
                            type="button"
                            role="radio"
                            aria-checked={wizard.path === option.id}
                            class="card border p-4 text-left transition-colors {wizard.path === option.id
                                ? 'border-primary bg-primary/10'
                                : 'border-base-300 hover:bg-base-200'}"
                            data-testid="new-ledger-path-{option.id}"
                            onclick={() => wizard.choosePath(option.id)}
                        >
                            <span class="font-semibold">{option.title}</span>
                            <span class="text-sm text-base-content/70">{option.blurb}</span>
                        </button>
                    {/each}
                </div>
                {#if desktop}
                    <p class="text-sm">
                        Just looking?
                        <button type="button" class="link link-primary" data-testid="new-ledger-sample" disabled={working} onclick={() => void trySample()}
                            >Try with sample data</button
                        >.
                    </p>
                {/if}
            {:else if wizard.step === "kind"}
                <div class="grid gap-3 sm:grid-cols-3" role="radiogroup" aria-label="Starting chart of accounts">
                    {#each KINDS as each (each)}
                        <button
                            type="button"
                            role="radio"
                            aria-checked={wizard.kind === each}
                            class="card border p-4 text-left transition-colors {wizard.kind === each
                                ? 'border-primary bg-primary/10'
                                : 'border-base-300 hover:bg-base-200'}"
                            data-testid="new-ledger-kind-{each}"
                            onclick={() => void wizard.chooseKind(each)}
                        >
                            <span class="font-semibold">{KIND_LABELS[each].title}</span>
                            <span class="text-sm text-base-content/70">{KIND_LABELS[each].blurb}</span>
                        </button>
                    {/each}
                </div>
            {:else if wizard.step === "source"}
                <label class="flex flex-col gap-1">
                    <span class="text-sm font-medium">The app you are coming from</span>
                    <select
                        class="select w-full"
                        data-testid="new-ledger-source"
                        value={wizard.source?.id ?? ""}
                        onchange={(event) => {
                            const app = SOURCE_APPS.find((entry) => entry.id === event.currentTarget.value);
                            if (app !== undefined) void wizard.chooseSource(app);
                        }}
                    >
                        <option value="" disabled>Choose…</option>
                        {#each SOURCE_APPS as app (app.id)}
                            <option value={app.id}>{app.label} ({app.format})</option>
                        {/each}
                    </select>
                </label>
                {#if wizard.source !== null}
                    <p class="text-sm text-base-content/70" data-testid="new-ledger-source-howto">{wizard.source.howTo}</p>
                    <label class="flex flex-col gap-1">
                        <span class="text-sm font-medium">The exported file</span>
                        <input
                            type="file"
                            class="file-input w-full"
                            data-testid="new-ledger-source-file"
                            accept={wizard.source.extensions.map((ext) => `.${ext}`).join(",")}
                            onchange={(event) => wizard.chooseFile(event.currentTarget.files?.[0] ?? null)}
                        />
                    </label>
                    {#if warning !== null}
                        <div role="status" class="alert text-sm alert-warning" data-testid="new-ledger-source-warning">{warning}</div>
                    {/if}
                    <p class="text-sm text-base-content/70">
                        Ledgeline makes an empty ledger first, then opens the Imports screen with this file, where you match its accounts to yours before
                        anything is written.
                    </p>
                {/if}
            {:else if wizard.step === "details"}
                <div class="grid gap-4 sm:grid-cols-2">
                    <label class="flex flex-col gap-1">
                        <span class="text-sm font-medium">Base currency</span>
                        <select
                            class="select w-full"
                            data-testid="new-ledger-currency"
                            value={wizard.currencyPreset}
                            onchange={(event) => wizard.setCurrencyPreset(event.currentTarget.value)}
                        >
                            {#each CURRENCY_PRESETS as preset (preset.id)}
                                <option value={preset.id}>{preset.label} ({preset.style.commodity})</option>
                            {/each}
                            <option value={CUSTOM_CURRENCY}>Other…</option>
                        </select>
                    </label>
                    <label class="flex flex-col gap-1">
                        <span class="text-sm font-medium">Fiscal year starts in</span>
                        <select
                            class="select w-full"
                            data-testid="new-ledger-fiscal"
                            value={wizard.fiscalYearStart}
                            onchange={(event) => wizard.setFiscalYearStart(Number(event.currentTarget.value))}
                        >
                            {#each MONTHS as month, index (month)}
                                <option value={index + 1}>{month}</option>
                            {/each}
                        </select>
                    </label>
                    {#if wizard.currencyPreset === CUSTOM_CURRENCY}
                        <label class="flex flex-col gap-1">
                            <span class="text-sm font-medium">Currency code or symbol</span>
                            <input
                                class="input w-full"
                                data-testid="new-ledger-custom-code"
                                placeholder="SEK"
                                value={wizard.customCode}
                                oninput={(event) => wizard.setCustomCode(event.currentTarget.value)}
                            />
                        </label>
                        <label class="flex flex-col gap-1">
                            <span class="text-sm font-medium">Decimal places</span>
                            <input
                                type="number"
                                min="0"
                                max="4"
                                class="input w-full"
                                value={wizard.customDecimals}
                                oninput={(event) => wizard.setCustomDecimals(Number(event.currentTarget.value))}
                            />
                        </label>
                    {/if}
                </div>
                <label class="label gap-2 text-sm">
                    <input
                        type="checkbox"
                        class="checkbox checkbox-sm"
                        checked={wizard.decimalComma}
                        onchange={(event) => wizard.setDecimalComma(event.currentTarget.checked)}
                    />
                    Write amounts with a decimal comma (1.000,00)
                </label>
                <p class="text-sm" data-testid="new-ledger-sample-amount">
                    Amounts will look like <code class="font-mono">{sampleAmount(wizard.currency)}</code>.
                    {#if wizard.blocker !== null}<span class="text-error">{wizard.blocker}</span>{/if}
                </p>
                <p class="text-xs text-base-content/60">
                    Ledgeline's reports use calendar years for now. The fiscal year is noted in your journal so you, and later versions, know it.
                </p>
            {:else if wizard.step === "accounts"}
                <AccountReview
                    accounts={wizard.accounts}
                    onRename={(index, name) => wizard.renameAccount(index, name)}
                    onRemove={(index) => wizard.removeAccount(index)}
                />
            {:else if wizard.step === "create"}
                <div class="flex flex-col gap-3">
                    <div class="flex flex-wrap items-center gap-3">
                        <button
                            type="button"
                            class="btn"
                            data-testid="new-ledger-pick-folder"
                            disabled={working || !desktop}
                            onclick={() => void wizard.pickFolder()}
                        >
                            {wizard.busy === "folder" ? "Choosing…" : wizard.folder === null ? "Choose folder…" : "Choose another folder…"}
                        </button>
                        {#if wizard.folder !== null}
                            <span class="text-sm" data-testid="new-ledger-folder">
                                In <strong>{wizard.folder.name}</strong>
                                {#if !wizard.folder.empty}<span class="text-error"> — not empty</span>{/if}
                            </span>
                        {/if}
                    </div>
                    <p class="text-sm text-base-content/70">
                        Choose an empty folder, or make a new one in the picker. Ledgeline writes <code>main.journal</code> and <code>accounts.journal</code> there
                        and never overwrites a file.
                    </p>
                    <label class="flex flex-col gap-1">
                        <span class="text-sm font-medium">Ledger name</span>
                        <input
                            class="input w-full"
                            data-testid="new-ledger-title"
                            placeholder="My books"
                            value={wizard.title}
                            oninput={(event) => wizard.setTitle(event.currentTarget.value)}
                        />
                        <span class="text-xs text-base-content/60">The first line of main.journal, and the name shown at the top of the app.</span>
                    </label>
                </div>
            {/if}

            {#if wizard.error !== null}
                <div role="alert" class="alert text-sm alert-error" data-testid="new-ledger-error">{wizard.error}</div>
            {/if}
        </div>
    </section>

    <footer class="flex items-center justify-between gap-2">
        <div>
            {#if wizard.step !== "start"}
                <button type="button" class="btn btn-ghost" disabled={working} onclick={() => wizard.back()}>Back</button>
            {:else if wizard.status?.journalOpen}
                <button type="button" class="btn btn-ghost" data-testid="new-ledger-cancel" onclick={onCancel}>Cancel</button>
            {/if}
        </div>
        <div class="flex items-center gap-3">
            {#if wizard.step !== "details" && wizard.blocker !== null && wizard.step !== "start"}
                <span class="text-sm text-base-content/60" data-testid="new-ledger-blocker">{wizard.blocker}</span>
            {/if}
            {#if wizard.step === "create"}
                <button
                    type="button"
                    class="btn btn-primary"
                    data-testid="new-ledger-create"
                    disabled={wizard.blocker !== null || working || !desktop}
                    onclick={() => void create()}
                >
                    {wizard.busy === "create" ? "Creating…" : wizard.path === "import" ? "Create and import" : "Create ledger"}
                </button>
            {:else}
                <button
                    type="button"
                    class="btn btn-primary"
                    data-testid="new-ledger-next"
                    disabled={wizard.blocker !== null || working}
                    onclick={() => void wizard.next()}
                >
                    Next
                </button>
            {/if}
        </div>
    </footer>
</div>
