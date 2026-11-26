// The Settings form's model (SETTINGS-PLAN §3, §4): what Save sends, the
// Default / Use the default states, the file name's chips, and the numbers.
import {describe, expect, it} from "vitest";
import {bpsToPercent, clampMatching, defaultMatching, percentToBps} from "./matching.js";
import {
    agingBuckets,
    baselineOf,
    changedKeys,
    changesOf,
    FILE_KEYS,
    flatten,
    formOf,
    isEmpty,
    localProblems,
    nameExample,
    nameOf,
    nameParts,
    offersDefault,
    SECTIONS,
    showsDefault,
    splitExtension,
    wholeNumber,
    withDefault,
    withField,
    withoutField,
    withValue,
} from "./settingsForm.js";
import {SETTING_KEYS, WORD_KEYS} from "./settings.js";
import {settingsPage} from "./testing/settingsFixtures.js";

const saved = defaultMatching();

describe("UNIT settingsForm", () => {
    it("lays every setting out once, in the plan's section order", () => {
        expect(SECTIONS.map((s) => s.title)).toEqual([
            "Company",
            "New invoices",
            "Accounts",
            "Payments",
            "Editing",
            "Dashboard",
            "Files and folders",
            "Books",
            "Wording",
        ]);
        const shown = SECTIONS.flatMap((s) => s.keys);
        expect(new Set(shown).size).toBe(shown.length);
        for (const key of [...SETTING_KEYS, ...WORD_KEYS.map((w) => `words.${w}`)]) expect(shown).toContain(key);
        expect(FILE_KEYS).toHaveLength(SETTING_KEYS.length + WORD_KEYS.length);
    });

    it("flattens the word lists to their settings' names", () => {
        expect(flatten({filename: "x", words: {total: ["Total"]}})).toEqual({filename: "x", "words.total": ["Total"]});
    });

    it("opens clean, and sends only what changed", () => {
        const base = baselineOf(settingsPage());
        const form = formOf(base);
        expect(isEmpty(changesOf(base, form, saved))).toBe(true);

        const edited = withValue(withValue(form, "dso-days", 60), "words.total", ["Gesamt"]);
        const changes = changesOf(base, edited, saved);
        expect(changes).toEqual({set: {"dso-days": 60, "words.total": ["Gesamt"]}, unset: [], matching: {}});
        expect([...changedKeys(changes)].sort()).toEqual(["dso-days", "words.total"]);
        // Back to what was read: clean again.
        expect(isEmpty(changesOf(base, withValue(edited, "dso-days", 90), saved))).toBe(false);
        expect(isEmpty(changesOf(base, withValue(withValue(edited, "dso-days", 90), "words.total", base.values["words.total"]), saved))).toBe(true);
    });

    it("Use the default removes a setting the file sets, and shows the default", () => {
        const base = baselineOf(settingsPage());
        const form = formOf(base);
        expect(showsDefault(base, form, "default-terms", saved)).toBe(false);
        expect(offersDefault(base, form, "default-terms", saved)).toBe(true);
        expect(showsDefault(base, form, "dso-days", saved)).toBe(true);
        expect(offersDefault(base, form, "dso-days", saved)).toBe(false);

        const reset = withDefault(withDefault(form, base, "default-terms"), base, "words.quantity");
        expect(reset.values["default-terms"]).toBeNull();
        expect(reset.values["words.quantity"]).toEqual(["Qty", "Quantity", "Hours", "Units"]);
        expect(changesOf(base, reset, saved)).toEqual({set: {}, unset: ["default-terms", "words.quantity"], matching: {}});
        expect(showsDefault(base, reset, "default-terms", saved)).toBe(true);
        // A setting the file doesn't set, reset after an edit: nothing to send.
        expect(isEmpty(changesOf(base, withDefault(withValue(form, "dso-days", 10), base, "dso-days"), saved))).toBe(true);
        // Editing it again takes it out of the reset.
        expect(changesOf(base, withValue(reset, "default-terms", 14), saved).set).toEqual({"default-terms": 14});
    });

    it("an emptied optional setting is removed, never written as nothing", () => {
        const base = baselineOf(settingsPage());
        const emptied = withValue(formOf(base), "default-terms", null);
        expect(changesOf(base, emptied, saved)).toEqual({set: {}, unset: ["default-terms"], matching: {}});
        expect(showsDefault(base, emptied, "default-terms", saved)).toBe(true);
    });

    it("keeps the shared matching settings apart, against their saved values", () => {
        const base = baselineOf(settingsPage());
        const form = withValue(formOf(base), "settlementToleranceBps", 250);
        expect(changesOf(base, form, saved)).toEqual({set: {}, unset: [], matching: {settlementToleranceBps: 250}});
        expect(showsDefault(base, form, "settlementToleranceBps", saved)).toBe(false);
        // Saved as 250 meanwhile: nothing left to send.
        expect(isEmpty(changesOf(base, form, {...saved, settlementToleranceBps: 250}))).toBe(true);
        expect(withDefault(form, base, "settlementToleranceBps").matching.settlementToleranceBps).toBe(300);
    });

    it("converts the tolerance between a percentage and basis points", () => {
        expect(bpsToPercent(300)).toBe(3);
        expect(bpsToPercent(250)).toBe(2.5);
        expect(bpsToPercent(5)).toBe(0.1);
        expect(percentToBps(2.5)).toBe(250);
        expect(percentToBps(0.25)).toBe(25);
        expect(percentToBps(Number.NaN)).toBeNull();
        expect(clampMatching("settlementToleranceBps", 99_999)).toBe(10_000);
        expect(clampMatching("settlementLeadDays", 4.6)).toBe(5);
        expect(clampMatching("settlementLookbackDays", "x")).toBe(180);
    });

    it("reads the file name as chips and text, and writes it back as it was", () => {
        const pattern = "{date} {customer} Invoice {invoice_num}.md";
        const {stem, extension} = splitExtension(pattern);
        expect(extension).toBe(".md");
        const parts = nameParts(stem);
        expect(parts).toEqual([
            {kind: "text", text: ""},
            {kind: "field", field: "date"},
            {kind: "text", text: " "},
            {kind: "field", field: "customer"},
            {kind: "text", text: " Invoice "},
            {kind: "field", field: "invoice_num"},
            {kind: "text", text: ""},
        ]);
        expect(nameOf(parts, extension)).toBe(pattern);
        expect(nameExample(parts, extension, {date: "2026-11-25", customer: "Acme", number: "1001"})).toBe("2026-11-25 Acme Invoice 1001.md");
    });

    it("adds a chip at the end and removes one, joining the text around it", () => {
        const parts = nameParts("Invoice {invoice_num}");
        const added = withField(parts, "date");
        expect(nameOf(added, ".md")).toBe("Invoice {invoice_num}{date}.md");
        const spaced = withField(nameParts("Invoice"), "invoice_num");
        expect(nameOf(spaced, "")).toBe("Invoice {invoice_num}");
        const removed = withoutField(nameParts("{date} {customer} Invoice {invoice_num}"), 3);
        expect(nameOf(removed, "")).toBe("{date}  Invoice {invoice_num}");
        // Not a chip: unchanged.
        expect(withoutField(parts, 0)).toEqual(parts);
    });

    it("reads numbers as typed, and marks what doesn't read", () => {
        expect(wholeNumber(" 30 ")).toBe(30);
        expect(wholeNumber("")).toBeNull();
        expect(wholeNumber("3.5")).toBeUndefined();
        expect(agingBuckets("30, 60 90")).toEqual([30, 60, 90]);
        expect(agingBuckets("30, sixty")).toBeUndefined();

        const base = baselineOf(settingsPage());
        const form = formOf(base);
        expect(localProblems(form)).toEqual({});
        const bad = [
            withValue(form, "aging-buckets", [60, 30]),
            withValue(form, "aging-buckets", "30, sixty"),
            withValue(form, "dso-days", 0),
            withValue(form, "default-terms", 4000),
            withValue(form, "filename", "{date} Invoice.md"),
            withValue(form, "invoices-dir", " "),
            withValue(form, "words.total", []),
        ].map((f) => Object.keys(localProblems(f)));
        expect(bad).toEqual([["aging-buckets"], ["aging-buckets"], ["dso-days"], ["default-terms"], ["filename"], ["invoices-dir"], ["words.total"]]);
    });
});
