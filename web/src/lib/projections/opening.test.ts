// What the Projections tab opens first, and whether the open file can be
// starred (plan 33). Pure, so exhaustively: every combination the page can
// hand these functions.

import {describe, expect, it} from "vitest";
import {canBeDefault, defaultBlocker, defaultConflict, effectiveDefault, NOT_A_SCENARIO_FILE, openingSteps} from "./opening";
import type {ProjectionFile, ProjectionIndex} from "./types";

const file = (id: string, over: Partial<ProjectionFile> = {}): ProjectionFile => ({
    id,
    label: id,
    name: null,
    created: null,
    updated: null,
    isProjection: id.startsWith("projection-"),
    sizeBytes: 1,
    writable: true,
    isDefault: false,
    ...over,
});

const index = (files: ProjectionFile[], warnings: string[] = []): ProjectionIndex => ({
    rootLabel: "books",
    editable: true,
    truncated: false,
    files,
    directories: [""],
    warnings,
    defaultSupported: true,
});

describe("UNIT effectiveDefault — the first marked file in listing order (D6)", () => {
    it("is null with nothing marked", () => {
        expect(effectiveDefault([file("projection-a.journal"), file("projection-b.journal")])).toBeNull();
        expect(effectiveDefault([])).toBeNull();
    });

    it("is the one marked file", () => {
        expect(effectiveDefault([file("projection-a.journal"), file("projection-b.journal", {isDefault: true})])?.id).toBe("projection-b.journal");
    });

    it("is the FIRST of two marked files, as the engine lists them", () => {
        const files = [file("projection-a.journal", {isDefault: true}), file("projection-b.journal", {isDefault: true})];
        expect(effectiveDefault(files)?.id).toBe("projection-a.journal");
    });
});

describe("UNIT openingSteps — default, then last, then (by omission) a seed", () => {
    const D = "projection-d.journal";
    const L = "projection-l.journal";
    const cases: [string, string | null, string | null, boolean, {id: string; source: string}[]][] = [
        ["no default, no last", null, null, true, []],
        ["no default, a last file", null, L, true, [{id: L, source: "last"}]],
        ["a default, no last", D, null, true, [{id: D, source: "default"}]],
        ["a default that is also the last", D, D, true, [{id: D, source: "default"}]],
        [
            "a default and a different last",
            D,
            L,
            true,
            [
                {id: D, source: "default"},
                {id: L, source: "last"},
            ],
        ],
        ["no file routes: nothing", null, null, false, []],
        ["no file routes, a last file: nothing", null, L, false, []],
        ["no file routes, a default: nothing", D, null, false, []],
        ["no file routes, both: nothing", D, D, false, []],
        ["no file routes, different: nothing", D, L, false, []],
    ];
    it.each(cases)("%s", (_, defaultId, lastId, available, want) => {
        expect(openingSteps(defaultId, lastId, available)).toEqual(want);
    });
});

describe("UNIT defaultConflict — the listing's warning, only when two files are marked", () => {
    const conflict = "2 scenario files are marked as the default (…); the first one opens. Star one to fix it.";
    it("is the last warning when two or more files are marked", () => {
        const files = [file("projection-a.journal", {isDefault: true}), file("projection-b.journal", {isDefault: true})];
        expect(defaultConflict(index(files, ["a.journal is a symbolic link and was skipped", conflict]))).toBe(conflict);
    });

    it("is null with one marked file, whatever the scan warned about", () => {
        expect(defaultConflict(index([file("projection-a.journal", {isDefault: true})], ["a.journal is a symbolic link and was skipped"]))).toBeNull();
        expect(defaultConflict(index([file("projection-a.journal")], []))).toBeNull();
    });
});

describe("UNIT defaultBlocker — why the star is disabled", () => {
    it("a seed has no file to mark", () => {
        expect(defaultBlocker(null, null, true)).toBe("Save this scenario to a file first.");
    });

    it("another journal is not a scenario file", () => {
        expect(defaultBlocker("budget.journal", file("budget.journal"), true)).toBe(NOT_A_SCENARIO_FILE);
    });

    it("a file the main journal includes cannot be written", () => {
        expect(defaultBlocker("projection-x.journal", file("projection-x.journal", {writable: false}), true)).toMatch(/part of your main journal/);
    });

    it("a server without an editor cannot write anything", () => {
        expect(defaultBlocker("projection-x.journal", file("projection-x.journal"), false)).toBe("Ledgeline can't write to this journal's folder");
    });

    it("a writable scenario file can be starred", () => {
        expect(defaultBlocker("projection-x.journal", file("projection-x.journal"), true)).toBeNull();
    });

    it("canBeDefault agrees: a writable projection file, and nothing else", () => {
        expect(canBeDefault(file("projection-x.journal"))).toBe(true);
        expect(canBeDefault(file("projection-x.journal", {writable: false}))).toBe(false);
        expect(canBeDefault(file("budget.journal"))).toBe(false);
    });
});
