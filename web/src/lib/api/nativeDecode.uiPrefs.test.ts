// `GET`/`PATCH /api/ui-prefs` (plan 38 WP-5) over the committed golden that
// crates/ledgeline-server/tests/native_wire_golden.rs replays byte for byte,
// so renaming a Rust key fails both suites at once.

import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";
import {defaultUiPrefs} from "#lib/settings/uiPrefs.js";
import {ApiShapeError} from "./client";
import {decodeUiPrefsState} from "./nativeDecode";

function golden(name: string): Record<string, unknown> {
    return JSON.parse(readFileSync(new URL(`../../../../fixtures/native/v1/${name}.json`, import.meta.url), "utf8")) as Record<string, unknown>;
}

describe("UNIT nativeDecode — UI preferences over the ui-prefs golden", () => {
    it("decodes the built-in defaults, kept in the settings files, with no local keys and no issues", () => {
        const state = decodeUiPrefsState(golden("ui-prefs"));
        expect(state.prefs).toEqual(defaultUiPrefs());
        expect(state.storage).toBe("files");
        expect(state.localKeys).toEqual([]);
        expect(state.issues).toEqual([]);
    });

    it("decodes the keys the ledger's own file sets", () => {
        expect(decodeUiPrefsState({...golden("ui-prefs"), localKeys: ["columns.status", "matchWindow"]}).localKeys).toEqual(["columns.status", "matchWindow"]);
    });

    it("decodes issues, whole-file and per key", () => {
        const body = {
            ...golden("ui-prefs"),
            issues: [
                {file: "ledgeline.toml", key: null, message: "is not valid TOML"},
                {file: "ui-prefs.toml", key: "matchWindow", message: "must be a whole number"},
            ],
        };
        expect(decodeUiPrefsState(body).issues).toEqual([
            {file: "ledgeline.toml", key: null, message: "is not valid TOML"},
            {file: "ui-prefs.toml", key: "matchWindow", message: "must be a whole number"},
        ]);
    });

    describe("refuses a body it cannot trust", () => {
        const withPrefs = (prefs: Record<string, unknown>) => {
            const body = golden("ui-prefs");
            return {...body, prefs: {...(body.prefs as Record<string, unknown>), ...prefs}};
        };
        const prefsWithout = (key: string) => {
            const body = golden("ui-prefs");
            const prefs = {...(body.prefs as Record<string, unknown>)};
            delete prefs[key];
            return {...body, prefs};
        };
        it.each([
            ["a missing key", prefsWithout("matchWindow")],
            ["a missing column", withPrefs({columns: {date: true}})],
            ["a mistyped flag", withPrefs({insightsOpen: "yes"})],
            ["an unknown tab", withPrefs({insightsTab: "gone"})],
            ["a fractional number", withPrefs({reviewMaxDays: 1.5})],
            ["an absent projection id", prefsWithout("lastProjectionId")],
            ["a non-array benchmark list", withPrefs({benchmarks: "SPY"})],
            ["an unknown storage", {...golden("ui-prefs"), storage: "cloud"}],
            ["the retired ledger storage", {...golden("ui-prefs"), storage: "ledger"}],
            ["no localKeys array", {...golden("ui-prefs"), localKeys: undefined}],
            ["a non-string local key", {...golden("ui-prefs"), localKeys: [3]}],
            ["no issues array", {...golden("ui-prefs"), issues: null}],
            ["not an object", []],
        ])("%s", (_label, body) => {
            expect(() => decodeUiPrefsState(body)).toThrow(ApiShapeError);
        });
    });
});
