<script lang="ts">
    // "How do I read this file?" — the top-level directives, with `date-format`
    // given the space it earns.
    //
    // `date-format` is the setting that fails silently: `%m/%d/%Y` and
    // `%d/%m/%Y` accept exactly the same bytes and disagree about what they
    // mean, so a wrong choice imports a year of transactions into the wrong
    // months and nothing complains. The fix is not a better text field, it is
    // showing what the pattern DOES to one known date — which is what the
    // catalogue's static examples and `strftimeExample` are for.
    //
    // `source` is shown and never edited. hledger's `source` accepts a `| CMD`
    // form that it runs through the SHELL on import, so a write path that could
    // set one would turn "edit a text file" into arbitrary code execution; the
    // engine refuses it structurally and this panel says so out loud.
    import {CUSTOM_OPTION, DATE_FORMATS, findDateFormat, strftimeExample} from "../dateFormats";
    import {settingText, withFlag, withSetting, type FormItem, type SettingKey} from "../model";
    import type {RulesSourcePref} from "../types";

    let {
        items,
        source,
        onChange,
        disabled,
    }: {
        items: FormItem[];
        /** The engine's own reading of the `source` line, including whether hledger would shell out. */
        source: RulesSourcePref | null;
        onChange: (items: FormItem[]) => void;
        disabled: boolean;
    } = $props();

    const dateFormat = $derived(settingText(items, "date-format") ?? "");
    /** The user asked for the custom field even though the current value is in the catalogue. */
    let forceCustom = $state(false);
    const customDate = $derived(forceCustom || (dateFormat !== "" && findDateFormat(dateFormat) === null));
    const dateExample = $derived(dateFormat === "" ? "" : (findDateFormat(dateFormat)?.example ?? strftimeExample(dateFormat)));

    const separator = $derived(settingText(items, "separator") ?? "");
    // `separator TAB` and `separator tab` both parse, so the membership test is
    // case-insensitive; a one-character separator is compared as itself.
    const KNOWN_SEPARATORS = [",", ";", "TAB", "SPACE", "|"];
    let forceCustomSeparator = $state(false);
    const customSeparator = $derived(forceCustomSeparator || (separator !== "" && !KNOWN_SEPARATORS.includes(separator.toUpperCase())));

    function set(key: SettingKey, value: string): void {
        onChange(withSetting(items, key, value));
    }
    function toggle(key: SettingKey, on: boolean): void {
        onChange(withFlag(items, key, on));
    }

    function onPickDateFormat(value: string): void {
        if (value === CUSTOM_OPTION) {
            forceCustom = true;
            return;
        }
        forceCustom = false;
        set("date-format", value);
    }

    function onPickSeparator(value: string): void {
        if (value === CUSTOM_OPTION) {
            forceCustomSeparator = true;
            return;
        }
        forceCustomSeparator = false;
        set("separator", value);
    }

    /** The extra settings this file happens to declare — shown so nothing in the file is invisible. */
    const EXTRA: SettingKey[] = ["currency", "encoding", "timezone"];
    const extras = $derived(EXTRA.filter((key) => settingText(items, key) !== null));
</script>

<div class="flex flex-col gap-4">
    <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <label class="form-control">
            <span class="label-text text-xs">Date format</span>
            <select
                class="select select-sm w-full"
                {disabled}
                aria-label="Date format"
                value={customDate ? CUSTOM_OPTION : dateFormat}
                onchange={(event) => onPickDateFormat(event.currentTarget.value)}
            >
                <option value="">Not set — hledger will guess</option>
                {#each DATE_FORMATS as option (option.pattern)}
                    <option value={option.pattern}>{option.label} — {option.example}</option>
                {/each}
                <option value={CUSTOM_OPTION}>Something else…</option>
            </select>
            {#if customDate}
                <input
                    type="text"
                    class="input input-sm mt-1 w-full font-mono"
                    {disabled}
                    aria-label="Custom date format"
                    placeholder="%d %b %Y"
                    value={dateFormat}
                    oninput={(event) => set("date-format", event.currentTarget.value)}
                />
            {/if}
            <span class="label-text-alt text-base-content/60 mt-1 text-xs">
                {#if dateFormat === ""}
                    Set this when the import mis-files dates — <code>03/04</code> is March 4th to some banks and April 3rd to others.
                {:else}
                    A date written this way looks like <code data-testid="date-format-example">{dateExample}</code>.
                {/if}
            </span>
        </label>

        <div class="grid grid-cols-2 gap-3">
            <label class="form-control">
                <span class="label-text text-xs">Header lines to skip</span>
                <input
                    type="number"
                    min="0"
                    class="input input-sm w-full"
                    {disabled}
                    aria-label="Header lines to skip"
                    value={settingText(items, "skip") ?? ""}
                    oninput={(event) => set("skip", event.currentTarget.value.trim())}
                />
            </label>
            <label class="form-control">
                <span class="label-text text-xs">Separator</span>
                <select
                    class="select select-sm w-full"
                    {disabled}
                    aria-label="Separator"
                    value={customSeparator ? CUSTOM_OPTION : separator}
                    onchange={(event) => onPickSeparator(event.currentTarget.value)}
                >
                    <option value="">Not set — comma</option>
                    <option value=",">Comma ,</option>
                    <option value=";">Semicolon ;</option>
                    <option value="TAB">Tab</option>
                    <option value="SPACE">Space</option>
                    <option value="|">Pipe |</option>
                    <option value={CUSTOM_OPTION}>Something else…</option>
                </select>
                {#if customSeparator}
                    <input
                        type="text"
                        class="input input-sm mt-1 w-full font-mono"
                        {disabled}
                        aria-label="Custom separator"
                        value={separator}
                        oninput={(event) => set("separator", event.currentTarget.value)}
                    />
                {/if}
            </label>
            <label class="form-control">
                <span class="label-text text-xs">Decimal mark</span>
                <select
                    class="select select-sm w-full"
                    {disabled}
                    aria-label="Decimal mark"
                    value={settingText(items, "decimal-mark") ?? ""}
                    onchange={(event) => set("decimal-mark", event.currentTarget.value)}
                >
                    <option value="">Not set</option>
                    <option value=".">Point 1234.56</option>
                    <option value=",">Comma 1234,56</option>
                </select>
            </label>
            <label class="form-control">
                <span class="label-text text-xs">Balance assertion</span>
                <select
                    class="select select-sm w-full"
                    {disabled}
                    aria-label="Balance assertion type"
                    value={settingText(items, "balance-type") ?? ""}
                    onchange={(event) => set("balance-type", event.currentTarget.value)}
                >
                    <option value="">Not set</option>
                    <option value="=">= partial</option>
                    <option value="==">== total</option>
                    <option value="=*">=* partial, subaccounts included</option>
                    <option value="==*">==* total, subaccounts included</option>
                </select>
            </label>
        </div>
    </div>

    <div class="flex flex-wrap gap-x-6 gap-y-2">
        <label class="label cursor-pointer justify-start gap-2 text-sm">
            <input
                type="checkbox"
                class="checkbox checkbox-sm"
                {disabled}
                checked={settingText(items, "newest-first") !== null}
                onchange={(event) => toggle("newest-first", event.currentTarget.checked)}
            />
            <span>Newest row first</span>
        </label>
        <label class="label cursor-pointer justify-start gap-2 text-sm">
            <input
                type="checkbox"
                class="checkbox checkbox-sm"
                {disabled}
                checked={settingText(items, "intra-day-reversed") !== null}
                onchange={(event) => toggle("intra-day-reversed", event.currentTarget.checked)}
            />
            <span>Same-day rows reversed</span>
        </label>
    </div>

    {#if extras.length > 0}
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
            {#each extras as key (key)}
                <label class="form-control">
                    <span class="label-text text-xs">{key}</span>
                    <input
                        type="text"
                        class="input input-sm w-full"
                        {disabled}
                        aria-label={key}
                        value={settingText(items, key) ?? ""}
                        oninput={(event) => set(key, event.currentTarget.value)}
                    />
                </label>
            {/each}
        </div>
    {/if}

    {#if source !== null}
        <div class="border-base-content/10 rounded-box border p-3" data-testid="imports-source">
            <div class="mb-1 flex items-center gap-2">
                <span class="badge badge-ghost badge-sm">read-only</span>
                <span class="text-xs font-medium">source</span>
            </div>
            <code class="text-base-content/80 block break-all text-xs">{source.value}</code>
            <p class="text-base-content/60 mt-2 text-xs">
                {#if source.executesShellCommand}
                    This <code>source</code> is a shell command: on <code>hledger import</code>, hledger runs it through your shell and reads what it prints.
                    Ledgeline never runs it, and will not write or change a <code>source</code> line — edit it in a terminal if you need to.
                {:else}
                    Names the data file this rules file reads. Ledgeline shows it but never rewrites it, because a <code>source</code> line can also carry a shell
                    command that hledger would run on import.
                {/if}
            </p>
        </div>
    {/if}
</div>
