// What each row of the rules-file list says — kept pure so it can be tested,
// since the component itself cannot be (see the vitest `node` project).
//
// Two things drive this. First, labels are NOT unique: a real ledger keeps
// `2025/imports/capitalone.csv.rules` alongside `2026/imports/capitalone.csv.rules`,
// and a list showing only the label shows the same row twice with no way to tell
// which is which. The id IS the relative path, so the directory it sits in is
// what disambiguates.
//
// Second, the per-file counts used to sit under the label, where they were busy
// and routinely truncated in a narrow pane. They move into a tooltip: still one
// hover (or one Tab) away, but no longer competing with the name.

import type {RulesFileSummary} from "./types";

export interface RulesFileRow {
    /**
     * The directory part of the id, with no trailing slash — `""` for a file
     * sitting directly in the journal's own folder, where there is nothing to
     * disambiguate and a blank second line would just add noise.
     */
    readonly directory: string;
    /** The file's own name, i.e. the id's last segment. */
    readonly fileName: string;
    /** The tooltip: the full relative path, then everything the row no longer shows. */
    readonly detail: string;
}

/** `3 rules, 1 advanced`, or why we cannot say. */
function counts(file: RulesFileSummary): string {
    if (!file.parsed) return "not readable";
    const rules = file.ifBlockCount === 1 ? "1 rule" : `${file.ifBlockCount} rules`;
    return file.opaqueItemCount === 0 ? rules : `${rules}, ${file.opaqueItemCount} advanced`;
}

/**
 * The accounts this file posts to, when it declares them. Worth a tooltip line:
 * it is the fastest way to tell two same-named files apart when they differ by
 * account rather than by year.
 */
function accounts(file: RulesFileSummary): string | null {
    if (!file.parsed) return null;
    if (file.account1 === null && file.account2 === null) return null;
    return `${file.account1 ?? "?"} → ${file.account2 ?? "?"}`;
}

/**
 * Split one discovered file into what the row shows and what the tooltip adds.
 *
 * The id is always forward-slash separated (the engine builds it that way,
 * relative to the scan root), so this never needs to know about platform
 * separators.
 */
export function fileRow(file: RulesFileSummary): RulesFileRow {
    const cut = file.id.lastIndexOf("/");
    const directory = cut === -1 ? "" : file.id.slice(0, cut);
    const fileName = cut === -1 ? file.id : file.id.slice(cut + 1);
    const parts = [file.id, counts(file), accounts(file)].filter((part): part is string => part !== null && part !== "");
    return {directory, fileName, detail: parts.join(" · ")};
}
