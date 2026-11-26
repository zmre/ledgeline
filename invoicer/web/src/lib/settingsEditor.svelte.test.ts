// Save on the Settings page (SETTINGS-PLAN §3, D1, D5): the checks before
// anything is written, each question it asks, the conflict with an outside
// edit, live validation and the read-only states.
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {EditorApiError} from "./editorApi.js";
import {defaultMatching, type Matching} from "./matching.js";
import type {SettingsCheck, SettingsSaved} from "./settings.js";
import {SettingsEditor} from "./settingsEditor.svelte.js";
import {settingsPage} from "./testing/settingsFixtures.js";

const CLEAN: SettingsCheck = {problems: [], effects: [], refused: null};

function harness(page = settingsPage()) {
    let matching: Matching = defaultMatching();
    const api = {
        fetch: vi.fn(() => Promise.resolve(page)),
        check: vi.fn((): Promise<SettingsCheck> => Promise.resolve(CLEAN)),
        save: vi.fn((): Promise<SettingsSaved> => Promise.resolve({revision: "rev-2", values: page.values, set: page.set, created: false})),
        matching: () => matching,
        saveMatching: vi.fn((values: Partial<Matching>) => {
            matching = {...matching, ...values};
            return Promise.resolve();
        }),
    };
    return {api, editor: new SettingsEditor(api, 50)};
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("COMPONENT SettingsEditor", () => {
    it("opens clean, and Save writes only what changed", async () => {
        const {api, editor} = harness();
        await editor.load();
        expect(editor.dirty).toBe(false);
        editor.set("dso-days", 60);
        editor.useDefault("words.quantity");
        expect(editor.dirty).toBe(true);
        expect([...editor.changed].sort()).toEqual(["dso-days", "words.quantity"]);

        expect(await editor.save()).toBe("saved");
        expect(api.check).toHaveBeenCalledWith({revision: "rev-1", set: {"dso-days": 60}, unset: ["words.quantity"]});
        expect(api.save).toHaveBeenCalledWith({revision: "rev-1", set: {"dso-days": 60}, unset: ["words.quantity"], createFile: false, confirmed: []});
        expect(api.saveMatching).not.toHaveBeenCalled();
        expect(editor.dirty).toBe(false);
        expect(editor.savedNote).toBe("Saved.");
    });

    it("saves the shared matching settings without touching the settings file", async () => {
        const {api, editor} = harness();
        await editor.load();
        editor.set("settlementLookbackDays", 365);
        expect(await editor.save()).toBe("saved");
        expect(api.saveMatching).toHaveBeenCalledWith({settlementLookbackDays: 365});
        expect(api.save).not.toHaveBeenCalled();
        expect(editor.dirty).toBe(false);
    });

    it("confirms a change that affects new bookings with each effect, then saves it confirmed", async () => {
        const {api, editor} = harness();
        await editor.load();
        const effect = {key: "payment-allocation" as const, message: "Payments you apply from now on are split pro rata."};
        api.check.mockResolvedValueOnce({...CLEAN, effects: [effect]});
        editor.set("payment-allocation", "pro-rata");

        expect(await editor.save()).toBe("asked");
        expect(editor.question).toEqual({kind: "confirm", effects: [effect]});
        expect(api.save).not.toHaveBeenCalled();

        expect(await editor.answer(true)).toBe("saved");
        expect(api.save).toHaveBeenLastCalledWith(expect.objectContaining({confirmed: ["payment-allocation"], set: {"payment-allocation": "pro-rata"}}));
    });

    it("cancelling the confirmation writes nothing and keeps the edit", async () => {
        const {api, editor} = harness();
        await editor.load();
        api.check.mockResolvedValueOnce({...CLEAN, effects: [{key: "revenue-account", message: "Rows book to revenue:other."}]});
        editor.set("revenue-account", "revenue:other");
        await editor.save();
        expect(await editor.answer(false)).toBe("failed");
        expect(api.save).not.toHaveBeenCalled();
        expect(editor.dirty).toBe(true);
    });

    it("refuses D5 outright, listing the deposits, and can undo just that change", async () => {
        const {api, editor} = harness();
        await editor.load();
        const refusal = {
            key: "cash-deposits" as const,
            message: "1 deposit is waiting for its final invoice.",
            deposits: [{number: "205", customer: "Contoso", date: "2026-10-01", waiting: "$1,500.00", bookedTo: ["revenue:consulting"]}],
        };
        api.check.mockResolvedValueOnce({...CLEAN, refused: refusal});
        editor.set("cash-deposits", "liability-until-final");
        editor.set("dso-days", 60);
        expect(await editor.save()).toBe("asked");
        expect(editor.question).toEqual({kind: "refused", refusal});
        editor.revert("cash-deposits");
        expect(await editor.answer(true)).toBe("failed");
        expect(api.save).not.toHaveBeenCalled();
        expect([...editor.changed]).toEqual(["dso-days"]);
    });

    it("asks before creating a settings file, then creates it", async () => {
        const {api, editor} = harness(settingsPage({file: "missing", set: [], revision: null}));
        await editor.load();
        editor.set("dso-days", 60);
        expect(await editor.save()).toBe("asked");
        expect(editor.question).toEqual({kind: "create"});
        expect(api.save).not.toHaveBeenCalled();
        expect(await editor.answer(true)).toBe("saved");
        expect(api.save).toHaveBeenLastCalledWith(expect.objectContaining({createFile: true, revision: null}));
    });

    it("asks to create the file when the server says there is none", async () => {
        const {api, editor} = harness();
        await editor.load();
        api.save.mockRejectedValueOnce(new EditorApiError(409, "no-file", "These books have no settings file yet."));
        editor.set("dso-days", 60);
        expect(await editor.save()).toBe("asked");
        expect(editor.question).toEqual({kind: "create"});
    });

    it("marks the server's problems by setting, and live while typing", async () => {
        const {api, editor} = harness();
        await editor.load();
        const problem = {key: "dso-days" as const, message: "The window must be at most 3660 days."};
        api.check.mockResolvedValue({...CLEAN, problems: [problem]});
        editor.set("dso-days", 3000);
        expect(editor.problems).toEqual({});
        await vi.advanceTimersByTimeAsync(50);
        expect(editor.problems).toEqual({"dso-days": problem.message});
        expect(await editor.save()).toBe("failed");
        expect(api.save).not.toHaveBeenCalled();
        expect(editor.saveError).toMatch(/marked below/);
    });

    it("keeps Save off while the page itself sees a problem", async () => {
        const {editor} = harness();
        await editor.load();
        editor.set("aging-buckets", "30, sixty");
        expect(editor.dirty).toBe(true);
        expect(editor.canSave).toBe(false);
        expect(Object.keys(editor.problems)).toEqual(["aging-buckets"]);
    });

    it("takes an outside change silently when nothing is unsaved", async () => {
        const {api, editor} = harness();
        await editor.load();
        api.fetch.mockResolvedValueOnce(settingsPage({revision: "rev-9", values: {...settingsPage().values, "dso-days": 45}}));
        await editor.refresh();
        expect(editor.form?.values["dso-days"]).toBe(45);
        expect(editor.conflict).toBeNull();
    });

    it("says the file changed under unsaved edits; Keep mine saves over it, Reload drops them", async () => {
        const {api, editor} = harness();
        await editor.load();
        editor.set("dso-days", 60);
        api.fetch.mockResolvedValueOnce(settingsPage({revision: "rev-9"}));
        await editor.refresh();
        expect(editor.conflict).toBe("rev-9");
        expect(editor.form?.values["dso-days"]).toBe(60);
        expect(await editor.save()).toBe("failed");

        expect(await editor.keepMine()).toBe("saved");
        expect(api.save).toHaveBeenLastCalledWith(expect.objectContaining({revision: "rev-9"}));

        editor.set("dso-days", 61);
        api.fetch.mockResolvedValueOnce(settingsPage({revision: "rev-10"}));
        await editor.refresh();
        await editor.reloadTheirs();
        expect(editor.dirty).toBe(false);
        expect(editor.conflict).toBeNull();
    });

    it("a refused save over a newer file is a conflict", async () => {
        const {api, editor} = harness();
        await editor.load();
        api.save.mockRejectedValueOnce(new EditorApiError(409, "changed", "The settings file changed since this page read it.", "rev-7"));
        editor.set("dso-days", 60);
        expect(await editor.save()).toBe("failed");
        expect(editor.conflict).toBe("rev-7");
    });

    it("this page's own save coming back isn't a conflict", async () => {
        const {api, editor} = harness();
        await editor.load();
        editor.set("dso-days", 60);
        await editor.save();
        editor.set("dso-days", 61);
        api.fetch.mockResolvedValueOnce(settingsPage({revision: "rev-2"}));
        await editor.refresh();
        expect(editor.conflict).toBeNull();
    });

    it("is read-only over an unusable or invalid file, and saves nothing", async () => {
        const {api, editor} = harness(settingsPage({file: "unusable", readOnly: true, revision: null, problem: "ledgeline.toml is a symbolic link."}));
        await editor.load();
        expect(editor.readOnly).toBe(true);
        editor.set("dso-days", 60);
        expect(editor.canSave).toBe(false);
        expect(await editor.save()).toBe("failed");
        expect(api.save).not.toHaveBeenCalled();
    });
});
