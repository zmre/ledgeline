// What the Projections tab opens on the first mount after a page load, and
// whether the open file can be starred as the default (plan 33). Pure: the
// page drives the stores; this only decides.
//
// # The default is the FIRST marked file in listing order (D6)
//
// The engine lists `projection-*` files first and by id, and reads the
// `; default-scenario: true` marker from those files only, so "first marked in
// listing order" is "first marked by id". Several marked files only come from
// hand edits or sync merges; the engine's listing then carries a warning, and
// this rule is what makes the outcome deterministic. The engine pins the same
// rule (`default_conflict`).

import type {ProjectionFile, ProjectionIndex} from "./types";

/** Where an opening attempt came from: the starred default, or the last file this browser opened. */
export type OpeningSource = "default" | "last";

/** One file to try opening, in order. */
export interface OpeningStep {
    id: string;
    source: OpeningSource;
}

/** The file the tab opens by default: the first marked file in listing order, or null. */
export function effectiveDefault(files: readonly ProjectionFile[]): ProjectionFile | null {
    return files.find((file) => file.isDefault) ?? null;
}

/**
 * The files to try on the first mount after a page load, in order (D2, D9).
 *
 * The default always comes first. The last-opened file follows it as the
 * fallback when the default cannot be opened, and is the whole of the list
 * when there is no default — today's behaviour. Empty when the engine has no
 * file routes at all, which seeds.
 */
export function openingSteps(defaultId: string | null, lastId: string | null, available: boolean): OpeningStep[] {
    if (!available) return [];
    const steps: OpeningStep[] = [];
    if (defaultId !== null) steps.push({id: defaultId, source: "default"});
    if (lastId !== null && lastId !== defaultId) steps.push({id: lastId, source: "last"});
    return steps;
}

/**
 * The listing's warning about several marked files, or null (D6).
 *
 * The engine appends that warning LAST, after the scan's own (see `index` in
 * `projections_api.rs`), and only when two or more files are marked — which is
 * what this checks, so a scan warning is never mistaken for it.
 */
export function defaultConflict(index: ProjectionIndex): string | null {
    const marked = index.files.filter((file) => file.isDefault).length;
    return marked >= 2 ? (index.warnings.at(-1) ?? null) : null;
}

/** The one sentence for "this file can't carry the marker because it isn't a scenario file" (D5). */
export const NOT_A_SCENARIO_FILE = "Only a scenario file Ledgeline saved (projection-….journal) can be the default. Use Save as to make one.";

/**
 * Whether a file can be the default (D5): one of Ledgeline's own scenario
 * files, and one the tab may write.
 */
export function canBeDefault(file: ProjectionFile): boolean {
    return file.isProjection && file.writable;
}

/**
 * Why the open scenario cannot be starred, or null when it can (D18, D5,
 * D20). The sentence is the star's tooltip.
 *
 * `file` is the open file's listing entry, or null when nothing was loaded
 * from a file (a seed) — or the listing does not have it yet.
 */
export function defaultBlocker(currentId: string | null, file: ProjectionFile | null, editable: boolean): string | null {
    if (currentId === null) return "Save this scenario to a file first.";
    if (file !== null && !file.isProjection) return NOT_A_SCENARIO_FILE;
    if (file !== null && !file.writable) return "This file is part of your main journal, so it can't be the default. Use Save as to make a scenario file.";
    if (!editable) return "Ledgeline can't write to this journal's folder";
    return null;
}
