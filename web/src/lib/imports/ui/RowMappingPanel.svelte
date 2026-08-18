<script lang="ts">
    // "Which column is which?" — the `fields` list, as a column→field table.
    //
    // A bare `fields date, description, amount` tells you nothing about whether
    // it is RIGHT. Labelling each row with the CSV's own header and a real
    // sample value (`Col 3 | Amount | -6.45`) turns checking the mapping from an
    // act of memory into an act of reading, which is the entire reason the
    // engine has a preview endpoint at all.
    //
    // When the preview is unavailable the reason is TYPED, so this says which
    // one it is rather than falling silently back to bare column numbers.
    //
    // The control is an `<input list>` over a `<datalist>`, not a `<select>`,
    // because a `fields` name is not drawn from a fixed set. Naming a column
    // with one of hledger's own field names assigns it; naming it ANYTHING else
    // labels it for interpolation, so `fields …, cat` is what makes
    // `comment category:%cat` possible. A dropdown can suggest the first kind
    // and cannot express the second. This is the same known-options-but-free-text
    // pattern `AccountInput.svelte` uses for account names.
    import {columnRoleHint, fieldNames, MAPPABLE_FIELDS, withFieldNames, type FormItem} from "../model";
    import type {PreviewUnavailable, RulesPreview} from "../types";

    let {
        items,
        preview,
        pending,
        onChange,
        disabled,
    }: {
        items: FormItem[];
        /** null = the preview REQUEST failed; `available: false` = the engine read nothing, and says why. */
        preview: RulesPreview | null;
        /**
         * A re-read is in flight after a save, so `preview` has been WITHHELD
         * rather than being null for the usual reason. Without this the two are
         * indistinguishable here and a refresh would announce itself as a
         * failure for as long as it took.
         */
        pending: boolean;
        onChange: (items: FormItem[]) => void;
        disabled: boolean;
    } = $props();

    const names = $derived(fieldNames(items));
    const columns = $derived(Math.max(names?.length ?? 0, preview?.available === true ? preview.columns : 0));
    const rows = $derived(Array.from({length: columns}, (_, index) => index));

    /** The header cell for a column, when the preview found one. */
    function header(index: number): string {
        return preview?.header?.[index] ?? "";
    }

    /** The first sample value for a column, when the preview read a row. */
    function sample(index: number): string {
        return preview?.rows[0]?.[index] ?? "";
    }

    function nameAt(index: number): string {
        return names?.[index] ?? "";
    }

    /**
     * Set column `index` to `field`, padding with ignored columns if the
     * `fields` list was shorter than the CSV.
     *
     * Padding with `""` rather than a guessed name is what keeps this honest:
     * an empty name in a `fields` list is exactly hledger's "ignore this
     * column", so a padded slot means what it looks like.
     *
     * Called on every keystroke with the RAW value and again on blur with a
     * trimmed one. Trimming per keystroke would eat the space the moment you
     * typed it, so a stray leading space is left to `validateForm` — which now
     * says which characters are allowed — and tidied when you leave the field.
     */
    function setColumn(index: number, field: string): void {
        const next = [...(names ?? [])];
        while (next.length <= index) next.push("");
        next[index] = field;
        onChange(withFieldNames(items, next));
    }

    /** The names the datalist SUGGESTS: the common fields, plus whatever this file already uses. */
    const options = $derived.by(() => {
        const used = (names ?? []).filter((name) => name !== "" && !MAPPABLE_FIELDS.includes(name));
        return [...MAPPABLE_FIELDS, ...new Set(used)];
    });

    // One datalist shared by every row (an id may back many inputs), with a
    // per-instance suffix so two mounted panels cannot collide — the same guard
    // `AccountInput.svelte` uses.
    const listId = `csvfield-${Math.random().toString(36).slice(2)}`;

    const UNAVAILABLE: Record<PreviewUnavailable, string> = {
        noDataFile: "this rules file does not name a data file to read",
        sourceIsCommand: "its `source` is a shell command, which Ledgeline will not run",
        sourceOutsideRoot: "its `source` points outside the journal's own directory",
        notRegularFile: "the file it names is not a regular file",
        unreadable: "the file it names could not be read",
        notUtf8: "the file it names is not valid UTF-8",
        empty: "the file it names is empty",
    };
    // `pending` first: a withheld preview is not a missing one, and saying "the
    // preview request failed" about a request still in flight would be a lie
    // that resolves itself, which is the hardest kind to notice.
    const unavailableReason = $derived(
        pending ? null : preview === null ? "the preview request failed" : preview.reason === null ? null : UNAVAILABLE[preview.reason]
    );
</script>

<div class="flex flex-col gap-3">
    {#if pending}
        <p class="text-base-content/60 text-xs" data-testid="imports-preview-pending">
            Re-reading the data file with the settings you just saved… the sample values below are held back until it answers.
        </p>
    {:else if preview?.available === true}
        <p class="text-base-content/60 text-xs">
            Sample values are from <code>{preview.dataLabel ?? "the data file"}</code>, split on
            <code>{preview.separator === "\t" ? "TAB" : preview.separator}</code>.
        </p>
    {:else if unavailableReason !== null}
        <p class="text-base-content/60 text-xs" data-testid="imports-no-preview">
            No sample rows to show — {unavailableReason}. The columns below are numbered instead.
        </p>
    {/if}

    {#if columns === 0}
        <p class="text-base-content/70 text-sm">
            This file has no <code>fields</code> line, and Ledgeline could not read its data file to guess one. Add the columns from a terminal, or open the CSV and
            come back.
        </p>
    {:else}
        <div class="border-base-content/10 rounded-box overflow-x-auto border">
            <table class="table-zebra table-sm table">
                <thead>
                    <tr>
                        <th class="w-16">Column</th>
                        <th>In the CSV</th>
                        <th>Example</th>
                        <th class="w-56">Field name</th>
                    </tr>
                </thead>
                <tbody>
                    {#each rows as index (index)}
                        <tr>
                            <td class="text-base-content/60">{index + 1}</td>
                            <td class="font-medium">{header(index) || "—"}</td>
                            <td class="text-base-content/70 font-mono text-xs">{sample(index) || "—"}</td>
                            <td>
                                <input
                                    type="text"
                                    class="input input-xs w-full"
                                    list={listId}
                                    {disabled}
                                    autocomplete="off"
                                    spellcheck="false"
                                    placeholder="(not imported)"
                                    aria-label="Field name for column {index + 1}"
                                    aria-describedby="{listId}-hint-{index}"
                                    value={nameAt(index)}
                                    oninput={(event) => setColumn(index, event.currentTarget.value)}
                                    onchange={(event) => setColumn(index, event.currentTarget.value.trim())}
                                />
                                <span id="{listId}-hint-{index}" class="text-base-content/50 block pt-0.5 text-xs">{columnRoleHint(nameAt(index))}</span>
                            </td>
                        </tr>
                    {/each}
                </tbody>
            </table>
        </div>
        <datalist id={listId}>
            {#each options as field (field)}
                <option value={field}></option>
            {/each}
        </datalist>
        <p class="text-base-content/60 text-xs">
            Every import needs a <code>date</code> and an amount (<code>amount</code>, or <code>amount-in</code> and <code>amount-out</code> for banks that use two
            columns). Leave a column blank to skip it.
        </p>
        <p class="text-base-content/60 text-xs">
            You are not limited to the suggested names. Any other name simply labels the column so later rules can use it — call one <code>cat</code> and you
            can write <code>comment category:%cat</code>.
        </p>
    {/if}
</div>
