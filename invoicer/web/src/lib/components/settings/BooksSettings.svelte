<!-- Settings → These books (SETTINGS-PLAN §4): one form, in sections, in the
     plan's order: Company · New invoices · Accounts · Payments · Editing ·
     Dashboard · Files and folders · Books (Ledgeline's, read-only) · Wording
     (advanced, folded). Save and Discard are the page's (top right); this
     draws the form and edits `editor`.

     From `lg` up, a rail of the sections on the left, a dot on each with
     unsaved changes; below `lg`, the same links as a wrapping row. In accrual
     books the cash-books choices fold into one line. A read-only page (an
     unusable or invalid settings file) says why and disables every control.
     The three payment-matching settings are shared with Ledgeline, each with
     "Use for these books only". -->
<script lang="ts">
    import AccountInput from "#ui/components/AccountInput.svelte";
    import PinToggle from "#ui/components/PinToggle.svelte";
    import {openInLedgeline} from "#lib/api.js";
    import {openBooksFile} from "#lib/customers.js";
    import {localToday, nextNumberReason} from "#lib/invoices.js";
    import {COMING_LATER, COMING_LATER_CHOICE, errorText, settingLabel, settingsMessage} from "#lib/labels.js";
    import {bpsToPercent, MATCHING_SPEC, percentToBps, type MatchingKey} from "#lib/matching.js";
    import type {ComingLater, SettingKey, SettingName} from "#lib/settings.js";
    import {SETTINGS_FILE} from "#lib/settingsApi.js";
    import type {SettingsEditor} from "#lib/settingsEditor.svelte.js";
    import {
        agingBuckets,
        CASH_ONLY_KEYS,
        CHOICES,
        isMatchingKey,
        matchingValue,
        offersDefault,
        SECTIONS,
        showsDefault,
        wholeNumber,
        type Section,
    } from "#lib/settingsForm.js";
    import type {UiPrefs} from "#lib/uiPrefs.svelte.js";
    import FileNameField from "./FileNameField.svelte";
    import SettingRow from "./SettingRow.svelte";
    import WordListField from "./WordListField.svelte";

    let {editor, prefs}: {editor: SettingsEditor; prefs: UiPrefs} = $props();

    const page = $derived(editor.page);
    const base = $derived(editor.base);
    const form = $derived(editor.form);
    const readOnly = $derived(editor.readOnly);
    const changed = $derived(editor.changed);
    const problems = $derived(editor.problems);
    const accrual = $derived(page?.books.basis === "accrual");
    const accounts = $derived(page?.choices.accounts ?? []);

    const value = (key: string): unknown => form?.values[key];
    const text = (key: string): string => {
        const v = value(key);
        return v === null || v === undefined ? "" : Array.isArray(v) ? v.join(", ") : String(v);
    };
    const matching = (key: MatchingKey): number => (form === null ? prefs.matching[key] : matchingValue(form, prefs.matching, key));

    /** The row's state, for SettingRow. */
    function row(key: string) {
        const isDefault = base !== null && form !== null && showsDefault(base, form, key, prefs.matching);
        const inferred = page !== null && Object.hasOwn(page.inferred, key) ? page.inferred[key as keyof typeof page.inferred] : null;
        return {
            setting: key,
            isDefault,
            found: inferred,
            offersDefault: !readOnly && base !== null && form !== null && offersDefault(base, form, key, prefs.matching),
            affects: page?.affectsBookings.includes(key as SettingKey) ?? false,
            changed: changed.has(key),
            problem: problems[key] === undefined ? null : settingsMessage(problems[key]),
            disabled: readOnly,
            onUseDefault: () => editor.useDefault(key),
        };
    }

    const sectionChanged = (section: Section): boolean => section.keys.some((k) => changed.has(k));

    /** The disabled "coming later" choices of `setting`, or the section's own rows. */
    const comingFor = (setting: string): ComingLater[] => (page?.coming ?? []).filter((c) => c.setting === setting);
    const comingIn = (section: "payments" | "editing"): ComingLater[] => (page?.coming ?? []).filter((c) => c.section === section && c.setting === null);
    const comingLabel = (c: ComingLater): string => COMING_LATER[c.id]?.label ?? c.label;

    // The file name's example, with the next number when the books have one.
    const sample = $derived({date: localToday(), customer: "Acme", number: page?.nextNumber?.number ?? "1001"});

    const companyTitle = (link: unknown): string => {
        if (typeof link !== "string") return "";
        return page?.choices.companies.find((c) => c.link === link)?.title ?? link.replace(/^\[\[|\]\]$/g, "");
    };

    let openFailure = $state<string | null>(null);
    async function openFile(): Promise<void> {
        openFailure = null;
        try {
            await openBooksFile(SETTINGS_FILE);
        } catch (e) {
            openFailure = `The settings file couldn't be opened here: ${settingsMessage(errorText(e))}`;
        }
    }
    let ledgelineFailure = $state<string | null>(null);
    async function openLedgeline(): Promise<void> {
        ledgelineFailure = null;
        try {
            await openInLedgeline();
        } catch (e) {
            ledgelineFailure = errorText(e);
        }
    }

    let pinBusy = $state(false);
    async function pin(key: MatchingKey, on: boolean): Promise<void> {
        pinBusy = true;
        try {
            await prefs.pin(key, on);
        } finally {
            pinBusy = false;
        }
    }

    const sectionId = (id: string): string => `settings-section-${id}`;
</script>

{#snippet sectionLinks(kind: "rail" | "row")}
    {#each SECTIONS as section (section.id)}
        <li>
            <a
                href="#{sectionId(section.id)}"
                class={kind === "rail" ? "flex items-center justify-between gap-2" : "link text-sm link-hover"}
                data-testid="settings-{kind}-{section.id}"
            >
                <span>{section.title}</span>
                {#if sectionChanged(section)}
                    <span class="size-2 rounded-full bg-primary" aria-hidden="true"></span><span class="sr-only"> (unsaved changes)</span>
                {/if}
            </a>
        </li>
    {/each}
{/snippet}

{#snippet textInput(key: string, id: string, placeholder = "")}
    <input
        {id}
        type="text"
        class="input w-full max-w-md input-sm"
        value={text(key)}
        {placeholder}
        disabled={readOnly}
        oninput={(e) => editor.set(key, e.currentTarget.value)}
        data-testid="input-{key}"
    />
{/snippet}

{#snippet daysInput(key: string, id: string, nullable: boolean)}
    <span class="flex items-center gap-2">
        <input
            {id}
            type="text"
            inputmode="numeric"
            class="input w-24 input-sm"
            value={text(key)}
            disabled={readOnly}
            oninput={(e) => {
                const n = wholeNumber(e.currentTarget.value);
                editor.set(key, n === undefined ? e.currentTarget.value : n === null && !nullable ? e.currentTarget.value : n);
            }}
            data-testid="input-{key}"
        />
        <span class="text-sm">days</span>
    </span>
{/snippet}

{#snippet choiceInput(key: string, id: string)}
    {@const coming = comingFor(key)}
    <div class="flex flex-col gap-1.5" role="radiogroup" aria-labelledby="{id}-legend" {id} tabindex="-1">
        <span id="{id}-legend" class="sr-only">{settingLabel(key)}</span>
        {#each CHOICES[key] ?? [] as choice (choice.value)}
            <label class="flex cursor-pointer items-start gap-2 text-sm">
                <input
                    type="radio"
                    class="radio mt-0.5 radio-sm"
                    name={key}
                    value={choice.value}
                    checked={value(key) === choice.value}
                    disabled={readOnly}
                    onchange={() => editor.set(key, choice.value)}
                    data-testid="choice-{key}-{choice.value}"
                />
                <span>{choice.label}</span>
            </label>
        {/each}
        {#each coming as c (c.id)}
            <label class="flex items-start gap-2 text-sm text-base-content/50">
                <input type="radio" class="radio mt-0.5 radio-sm" name={key} disabled data-testid="coming-{c.id}" />
                <span>{comingLabel(c)} <span class="badge badge-ghost badge-xs">Coming later</span></span>
            </label>
        {/each}
    </div>
{/snippet}

{#snippet accountInput(key: string, id: string)}
    <div class="w-full max-w-md" {id}>
        <AccountInput
            bind:value={() => text(key), (v: string) => editor.set(key, v.trim() === "" ? null : v)}
            accountNames={accounts}
            label={settingLabel(key)}
            placeholder={page?.inferred[key as keyof typeof page.inferred] ?? "None"}
            disabled={readOnly}
        />
    </div>
{/snippet}

{#snippet matchingPin(key: MatchingKey)}
    {#if prefs.canPin}
        <PinToggle
            pinned={prefs.isPinned(key)}
            busy={pinBusy || readOnly}
            label="Use for these books only"
            onChange={(on) => void pin(key, on)}
            testid="pin-{key}"
        />
    {/if}
{/snippet}

{#snippet matchingInput(key: MatchingKey, id: string)}
    {@const spec = MATCHING_SPEC[key]}
    {#if key === "settlementToleranceBps"}
        <span class="flex items-center gap-2">
            <input
                {id}
                type="number"
                class="input w-24 input-sm"
                min={spec.min / 100}
                max={spec.max / 100}
                step="0.1"
                value={bpsToPercent(matching(key))}
                disabled={readOnly}
                onchange={(e) => {
                    const bps = percentToBps(e.currentTarget.valueAsNumber);
                    const kept = bps === null ? matching(key) : Math.min(spec.max, Math.max(spec.min, bps));
                    editor.set(key, kept);
                    e.currentTarget.value = String(bpsToPercent(kept));
                }}
                data-testid="input-{key}"
            />
            <span class="text-sm">% short</span>
        </span>
    {:else}
        <span class="flex items-center gap-2">
            <input
                {id}
                type="number"
                class="input w-24 input-sm"
                min={spec.min}
                max={spec.max}
                step="1"
                value={matching(key)}
                disabled={readOnly}
                onchange={(e) => {
                    const n = e.currentTarget.valueAsNumber;
                    const kept = Number.isFinite(n) ? Math.min(spec.max, Math.max(spec.min, Math.round(n))) : matching(key);
                    editor.set(key, kept);
                    e.currentTarget.value = String(kept);
                }}
                data-testid="input-{key}"
            />
            <span class="text-sm">{key === "settlementLookbackDays" ? "days after the invoice" : "days before the invoice"}</span>
        </span>
    {/if}
{/snippet}

{#snippet settingRow(key: string)}
    {@const r = row(key)}
    {#if isMatchingKey(key)}
        <SettingRow {...r}>
            {#snippet control(id)}{@render matchingInput(key, id)}{/snippet}
            {#snippet extra()}{@render matchingPin(key)}{/snippet}
        </SettingRow>
    {:else if key === "company"}
        <SettingRow {...r}>
            {#snippet control(id)}
                <select
                    {id}
                    class="select w-full max-w-md select-sm"
                    value={text(key)}
                    disabled={readOnly}
                    onchange={(e) => editor.set(key, e.currentTarget.value === "" ? null : e.currentTarget.value)}
                    data-testid="input-company"
                >
                    <option value="">None chosen</option>
                    {#if text(key) !== "" && !(page?.choices.companies ?? []).some((c) => c.link === text(key))}
                        <option value={text(key)}>{companyTitle(value(key))}</option>
                    {/if}
                    {#each page?.choices.companies ?? [] as c (c.link)}<option value={c.link}>{c.title}</option>{/each}
                </select>
            {/snippet}
        </SettingRow>
    {:else if key === "filename"}
        <SettingRow {...r}>
            {#snippet control(id)}
                <FileNameField {id} value={text(key)} {sample} disabled={readOnly} onChange={(v) => editor.set(key, v)} />
            {/snippet}
        </SettingRow>
    {:else if key === "number-start"}
        {@const next = page?.nextNumber ?? null}
        <SettingRow {...r}>
            {#snippet control(id)}
                <input
                    {id}
                    type="text"
                    class="input w-40 input-sm"
                    value={text(key)}
                    disabled={readOnly}
                    oninput={(e) => editor.set(key, e.currentTarget.value.trim() === "" ? null : e.currentTarget.value.trim())}
                    data-testid="input-number-start"
                />
            {/snippet}
            {#snippet extra()}
                {#if next !== null && next.source !== "start"}
                    {@const reason = nextNumberReason(next.after, next.source)}
                    <p class="text-sm text-base-content/70" data-testid="next-number">
                        Next number: <span class="font-medium">{next.number}</span>{reason === null ? "" : `, ${reason}`}. This applies only before the first
                        invoice.
                    </p>
                {/if}
            {/snippet}
        </SettingRow>
    {:else if key === "default-terms"}
        <SettingRow {...r}>
            {#snippet control(id)}{@render daysInput(key, id, true)}{/snippet}
        </SettingRow>
    {:else if key === "default-template"}
        {@const templates = (page?.choices.templates ?? []).filter((t) => t.kinds.length === 0 || t.kinds.includes("invoice"))}
        <SettingRow {...r}>
            {#snippet control(id)}
                <select
                    {id}
                    class="select w-full max-w-md select-sm"
                    value={text(key)}
                    disabled={readOnly}
                    onchange={(e) => editor.set(key, e.currentTarget.value === "" ? null : e.currentTarget.value)}
                    data-testid="input-default-template"
                >
                    <option value="">The first template for invoices</option>
                    {#each templates as t (t.value)}<option value={t.value}>{t.name}{t.starter ? " (a starter, copied in when first used)" : ""}</option>{/each}
                </select>
            {/snippet}
        </SettingRow>
    {:else if key.endsWith("-account")}
        <SettingRow {...r}>
            {#snippet control(id)}{@render accountInput(key, id)}{/snippet}
        </SettingRow>
    {:else if key in CHOICES}
        <SettingRow {...r}>
            {#snippet control(id)}{@render choiceInput(key, id)}{/snippet}
        </SettingRow>
    {:else if key === "aging-buckets"}
        <SettingRow {...r}>
            {#snippet control(id)}
                <input
                    {id}
                    type="text"
                    class="input w-48 input-sm"
                    value={text(key)}
                    disabled={readOnly}
                    oninput={(e) => editor.set(key, agingBuckets(e.currentTarget.value) ?? e.currentTarget.value)}
                    data-testid="input-aging-buckets"
                />
            {/snippet}
        </SettingRow>
    {:else if key === "dso-days" || key === "paid-window-days"}
        <SettingRow {...r}>
            {#snippet control(id)}{@render daysInput(key, id, false)}{/snippet}
        </SettingRow>
    {:else if key === "words.customer-tag"}
        <SettingRow {...r}>
            {#snippet control(id)}{@render textInput(key, id)}{/snippet}
        </SettingRow>
    {:else if key.startsWith("words.")}
        <SettingRow {...r}>
            {#snippet control(id)}
                <WordListField
                    {id}
                    value={Array.isArray(value(key)) ? (value(key) as string[]) : []}
                    disabled={readOnly}
                    onChange={(v) => editor.set(key, v)}
                />
            {/snippet}
        </SettingRow>
    {:else}
        <SettingRow {...r}>
            {#snippet control(id)}{@render textInput(key, id)}{/snippet}
        </SettingRow>
    {/if}
{/snippet}

{#snippet comingRow(c: ComingLater)}
    {@const words = COMING_LATER[c.id]}
    <div class="grid gap-x-6 gap-y-2 border-b border-base-300 py-3 last:border-b-0 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]" data-testid="coming-row-{c.id}">
        <div class="font-medium">{words?.label ?? c.label}</div>
        <div class="flex flex-col gap-1.5 text-sm" role="radiogroup" aria-label={words?.label ?? c.label}>
            <label class="flex items-start gap-2">
                <input type="radio" class="radio mt-0.5 radio-sm" checked disabled name="coming-{c.id}" />
                <span>{words?.now ?? "As now"}</span>
            </label>
            <label class="flex items-start gap-2 text-base-content/50">
                <input type="radio" class="radio mt-0.5 radio-sm" disabled name="coming-{c.id}" data-testid="coming-{c.id}" />
                <span>{COMING_LATER_CHOICE[c.id] ?? c.label} <span class="badge badge-ghost badge-xs">Coming later</span></span>
            </label>
        </div>
    </div>
{/snippet}

{#if page !== null && form !== null}
    <div class="flex flex-col gap-4 lg:flex-row lg:items-start lg:gap-8" data-testid="settings-books">
        <nav aria-label="Settings sections" class="hidden shrink-0 lg:sticky lg:top-16 lg:block lg:w-48" data-testid="settings-rail">
            <ul class="menu w-full menu-sm p-0">{@render sectionLinks("rail")}</ul>
        </nav>
        <nav aria-label="Settings sections" class="lg:hidden" data-testid="settings-section-row">
            <ul class="flex flex-wrap gap-x-4 gap-y-1">{@render sectionLinks("row")}</ul>
        </nav>

        <div class="flex min-w-0 flex-1 flex-col gap-6">
            {#if readOnly}
                <div role="alert" class="alert flex-col items-start alert-soft alert-warning" data-testid="settings-read-only">
                    <p class="font-medium">These settings can't be changed here.</p>
                    {#if page.problem !== null}<p class="text-sm">{settingsMessage(page.problem)}</p>{/if}
                    {#if page.problems.length > 0}
                        <ul class="list-disc pl-5 text-sm" data-testid="settings-file-problems">
                            {#each page.problems as p, at (at)}
                                <li>{p.key === null ? "" : `${settingLabel(p.key)}: `}{settingsMessage(p.message)}</li>
                            {/each}
                        </ul>
                    {/if}
                    <button type="button" class="btn btn-sm" onclick={() => void openFile()} data-testid="settings-open-file">Open the settings file</button>
                    {#if openFailure !== null}<p class="text-sm" role="status">{openFailure}</p>{/if}
                </div>
            {:else if page.file === "missing"}
                <p class="text-sm text-base-content/70" data-testid="settings-no-file">
                    These books have no settings file yet, so every setting is its default. Saving a change creates it, after asking.
                </p>
            {/if}
            {#if page.booksError !== null}
                <div role="alert" class="alert alert-soft text-sm alert-error" data-testid="settings-books-error">{settingsMessage(page.booksError)}</div>
            {/if}

            {#each SECTIONS as section (section.id)}
                <section id={sectionId(section.id)} class="scroll-mt-16" aria-labelledby="{sectionId(section.id)}-title" data-testid="section-{section.id}">
                    {#if section.id === "wording"}
                        <details class="group" data-testid="settings-wording">
                            <summary class="cursor-pointer text-lg font-semibold" id="{sectionId(section.id)}-title">
                                Wording <span class="text-sm font-normal text-base-content/60">(advanced)</span>
                            </summary>
                            <p class="mt-1 text-sm text-base-content/70">
                                The words the app recognizes in tables, total rows, tags and contact labels. The first in each list is the one it writes.
                            </p>
                            <div class="mt-2">
                                {#each section.keys as key (key)}{@render settingRow(key)}{/each}
                            </div>
                        </details>
                    {:else}
                        <h2 class="text-lg font-semibold" id="{sectionId(section.id)}-title">{section.title}</h2>
                        {#if section.id === "books"}
                            <p class="text-sm text-base-content/70">These are set in Ledgeline.</p>
                            <dl class="mt-1">
                                {#each section.keys as key (key)}
                                    <div class="grid gap-x-6 gap-y-1 border-b border-base-300 py-3 lg:grid-cols-[minmax(0,2fr)_minmax(0,3fr)]">
                                        <dt class="font-medium">{settingLabel(key)}</dt>
                                        <dd class="text-sm" data-testid="books-{key.replace('books.', '')}">
                                            {#if key === "books.basis"}
                                                {page.books.basis === null
                                                    ? "Not known until the books open"
                                                    : page.books.basis === "cash"
                                                      ? "Cash"
                                                      : "Accrual"}
                                            {:else}
                                                {page.books.closedThrough ?? "Open: no period is closed"}
                                            {/if}
                                        </dd>
                                    </div>
                                {/each}
                            </dl>
                            <button type="button" class="btn mt-2 btn-outline btn-sm" onclick={() => void openLedgeline()} data-testid="settings-open-ledgeline"
                                >Open in Ledgeline</button
                            >
                            {#if ledgelineFailure !== null}<p class="mt-1 text-sm text-error" role="status">{ledgelineFailure}</p>{/if}
                        {:else}
                            <div class="mt-1">
                                {#if section.id === "payments" && accrual}
                                    <p class="border-b border-base-300 py-3 text-sm text-base-content/70" data-testid="settings-accrual-note">
                                        These books use accrual; the cash-books choices don't apply.
                                    </p>
                                {/if}
                                {#each section.keys.filter((k) => !(accrual && CASH_ONLY_KEYS.includes(k))) as key (key)}
                                    {@render settingRow(key as SettingName)}
                                {/each}
                                {#if section.id === "payments" || section.id === "editing"}
                                    {#each comingIn(section.id) as c (c.id)}{@render comingRow(c)}{/each}
                                {/if}
                            </div>
                            {#if section.id === "payments"}
                                <p class="mt-2 text-xs text-base-content/60" data-testid="settings-matching-shared">
                                    How far back to look, the tolerance and payment before invoice are shared with Ledgeline{prefs.canPin
                                        ? ", unless kept for these books only"
                                        : ""}.
                                </p>
                            {/if}
                        {/if}
                    {/if}
                </section>
            {/each}
        </div>
    </div>
{/if}
