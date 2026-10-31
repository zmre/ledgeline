// The review edits an import is carrying (plan 32 D23, D24): overrides to the
// transactions it would write, keyed by where they sit in the import, and
// edits to similar transactions already in the journal, keyed by journal
// index. Nothing here is written anywhere — the dry run applies the overrides
// to what it shows, and the commit applies both, in the same write as the
// append (or the QuickBooks additions).
//
// One holder per import store (`importStore.svelte.ts` for CSV,
// `qbJournalStore.svelte.ts` for QuickBooks); the decisions are
// `reviewModel.ts`'s.

/* eslint-disable svelte/prefer-svelte-reactivity -- both maps are immutable
   snapshots replaced wholesale (`$state.raw`), never mutated in place. */
import type {EditResult} from "#lib/api/editFailure.js";
import type {WireBatchEdit, WireQbOverride, WireStagedOverride} from "#lib/api/native.js";
import {mergeOps, refKey, replayOverrides} from "./reviewModel";
import type {ReplayReport, StagedOverride, StagedRef, StagedSave, StagedTxn} from "./reviewTypes";

export type {StagedSave} from "./reviewTypes";

export interface StagedEdits {
    /** Every held override, in the order first made. */
    readonly overrides: readonly StagedOverride[];
    /** Every queued journal edit, in the order first made. */
    readonly journalEdits: readonly WireBatchEdit[];
    readonly overrideCount: number;
    readonly journalEditCount: number;
    /** Overrides a replay found changed underneath them ("check this one again"). */
    readonly flagged: readonly StagedOverride[];
    /** The override held for `ref`, if any. */
    overrideFor(ref: StagedRef): StagedOverride | null;
    /** Take one review save: merged into what is held for each transaction. */
    stage(save: StagedSave): EditResult;
    /** Let go of the override held for `ref` (the panel's "Discard" on a flagged edit). */
    discard(ref: StagedRef): void;
    /** Re-key, prune or drop the held overrides against a freshly generated import (D25). */
    replay(fresh: readonly StagedTxn[]): ReplayReport;
    /** The CSV wire form (`ordinal`). QuickBooks overrides are left out. */
    csvOverrides(): WireStagedOverride[];
    /** The QuickBooks wire form (`id`). CSV overrides are left out. */
    qbOverrides(): WireQbOverride[];
    /** Forget everything: a new stage, or a commit that landed. */
    clear(): void;
}

export function createStagedEdits(): StagedEdits {
    let overrides = $state.raw<ReadonlyMap<string, StagedOverride>>(new Map());
    let journalEdits = $state.raw<ReadonlyMap<number, WireBatchEdit>>(new Map());

    return {
        get overrides(): readonly StagedOverride[] {
            return [...overrides.values()];
        },
        get journalEdits(): readonly WireBatchEdit[] {
            return [...journalEdits.values()];
        },
        get overrideCount(): number {
            return overrides.size;
        },
        get journalEditCount(): number {
            return journalEdits.size;
        },
        get flagged(): readonly StagedOverride[] {
            return [...overrides.values()].filter((override) => override.flagged);
        },
        overrideFor(ref: StagedRef): StagedOverride | null {
            return overrides.get(refKey(ref)) ?? null;
        },

        stage(save: StagedSave): EditResult {
            const missing = save.overrides.find((entry) => entry.item.fingerprint === null);
            if (missing !== undefined) return {ok: false, failure: {kind: "unavailable", message: "This import's transactions cannot be edited."}};
            const nextOverrides = new Map(overrides);
            for (const {item, ops} of save.overrides) {
                if (ops.length === 0) continue;
                const key = refKey(item.ref);
                const held = nextOverrides.get(key);
                nextOverrides.set(
                    key,
                    Object.freeze(
                        held === undefined
                            ? {ref: item.ref, fingerprint: item.fingerprint as string, ops: Object.freeze([...ops]), original: item, flagged: false}
                            : // Saved again: the held fingerprint and original still name the
                              // generated text the whole edit applies to. A re-save is the user
                              // looking at it again, so it clears the flag.
                              {...held, ops: Object.freeze(mergeOps(held.ops, ops)), flagged: false}
                    )
                );
            }
            const nextEdits = new Map(journalEdits);
            for (const edit of save.journalEdits) {
                if (edit.ops.length === 0) continue;
                const held = nextEdits.get(edit.index);
                // The FIRST fingerprint is kept: the ops are cumulative over the
                // text the journal held when the first was made.
                nextEdits.set(edit.index, held === undefined ? {...edit, ops: [...edit.ops]} : {...held, ops: mergeOps(held.ops, edit.ops)});
            }
            overrides = nextOverrides;
            journalEdits = nextEdits;
            return {ok: true};
        },

        discard(ref: StagedRef): void {
            const key = refKey(ref);
            if (!overrides.has(key)) return;
            const next = new Map(overrides);
            next.delete(key);
            overrides = next;
        },

        replay(fresh: readonly StagedTxn[]): ReplayReport {
            const report = replayOverrides([...overrides.values()], fresh);
            overrides = new Map(report.kept.map((override) => [refKey(override.ref), override]));
            return report;
        },

        csvOverrides(): WireStagedOverride[] {
            return [...overrides.values()].flatMap((override) =>
                override.ref.kind === "csv" ? [{ordinal: override.ref.ordinal, fingerprint: override.fingerprint, ops: [...override.ops]}] : []
            );
        },
        qbOverrides(): WireQbOverride[] {
            return [...overrides.values()].flatMap((override) =>
                override.ref.kind === "qb" ? [{id: override.ref.id, fingerprint: override.fingerprint, ops: [...override.ops]}] : []
            );
        },
        clear(): void {
            overrides = new Map();
            journalEdits = new Map();
        },
    };
}
