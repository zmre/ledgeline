// Friendly field names (Phase 4 feedback H1, J10).
import {describe, expect, it} from "vitest";
import {
    COMING_LATER,
    COMING_LATER_CHOICE,
    errorText,
    fieldLabel,
    fieldList,
    friendlyMessage,
    SETTINGS,
    settingLabel,
    settingsMessage,
    snippetFieldList,
} from "./labels.js";
import {MATCHING_KEYS} from "./matching.js";
import {SETTING_KEYS, WORD_KEYS} from "./settings.js";
import {CHOICES, NAME_FIELDS, SECTIONS} from "./settingsForm.js";

/** Every setting the page names: the file's, the word lists, the shared matching ones and Ledgeline's books. */
const ALL_SETTINGS = [...SETTING_KEYS, ...WORD_KEYS.map((w) => `words.${w}`), ...MATCHING_KEYS, "books.basis", "books.closed-through"];

/** A setting's name (`invoices-dir`, `settlementLeadDays`, `words.total`), the file's syntax, or a placeholder. */
function leaks(text: string): string | null {
    if (text.includes("[[") || text.includes("{") || text.includes("[invoicer") || text.includes(".toml")) return text;
    for (const key of [...ALL_SETTINGS, ...WORD_KEYS]) {
        // A key with a hyphen, a dot or a capital inside is never an English word.
        if (/[-.A-Z]/.test(key.slice(1)) && text.includes(key)) return `${key} in “${text}”`;
    }
    return null;
}

describe("UNIT labels", () => {
    it("names a field by its label, else its key in words", () => {
        expect(fieldLabel("invoicee_email")).toBe("Invoicee email");
        expect(fieldLabel("revenue_account")).toBe("Revenue account");
        expect(fieldLabel("deposit_for")).toBe("Deposit for note");
        expect(fieldLabel("po_number")).toBe("Po number");
        expect(fieldList(["terms", "phones", "ledger_customer"])).toBe("Terms, Phones, Name in the books");
    });

    it("names a snippet's changed fields as its form does", () => {
        expect(snippetFieldList(["snippet_kind"])).toBe("Kind");
        expect(snippetFieldList(["title", "aliases", "body"])).toBe("Title, Aliases, Text");
    });

    it("reads a caught error through friendlyMessage", () => {
        expect(errorText(new Error("`invoice_num` can't change once 1042 is sent"))).toBe("Number can't change once 1042 is sent");
        expect(errorText("remove it from their `snippets:` first")).toBe("Remove it from their default snippets first");
    });

    it("rewrites keys in a server message, leaving other words", () => {
        expect(friendlyMessage("`invoicee_email` is missing")).toBe("Invoicee email is missing");
        expect(friendlyMessage("revenue_account: can't hold two spaces")).toBe("Revenue account: can't hold two spaces");
        expect(friendlyMessage("the name can't be empty")).toBe("The name can't be empty");
        expect(friendlyMessage("customers/Contoso.md changed on disk")).toBe("customers/Contoso.md changed on disk");
        expect(friendlyMessage("")).toBe("");
    });

    it("reads a problem's keys in a sentence", () => {
        expect(friendlyMessage("`due_date` 2026-10-01 is before `date` 2026-11-01")).toBe("Due date 2026-10-01 is before date 2026-11-01");
        expect(friendlyMessage("`po_num` is now `reference`")).toBe("PO number is now reference");
        expect(friendlyMessage("`deposit_for` must be text, not a list or map")).toBe("Deposit for note must be text, not a list or map");
    });

    it("quotes a key's value and any other code", () => {
        expect(friendlyMessage("`status: sending` isn't one of draft, sent, void")).toBe("Status “sending” isn't one of draft, sent, void");
        expect(friendlyMessage("`invoice_num` can't hold a comma: it is the journal's `invoice:` tag")).toBe(
            "Number can't hold a comma: it is the journal's “invoice:” tag"
        );
        expect(friendlyMessage("`customer: Customer X` isn't a [[wikilink]]")).toBe("Customer “Customer X” isn't a [[wikilink]]");
        expect(friendlyMessage("`unknown_key` is odd")).toBe("“unknown_key” is odd");
    });

    it("gives every setting a label and one help sentence", () => {
        for (const key of ALL_SETTINGS) {
            expect(SETTINGS[key]?.label, key).toBeTruthy();
            expect(SETTINGS[key]?.help, key).toMatch(/^[A-Z0-9].*\.$/);
            expect(settingLabel(key)).toBe(SETTINGS[key].label);
        }
        for (const section of SECTIONS) for (const key of section.keys) expect(Object.hasOwn(SETTINGS, key), key).toBe(true);
    });

    it("never shows a setting's name, the file's syntax or a placeholder", () => {
        const shown = [
            ...Object.values(SETTINGS).flatMap((s) => [s.label, s.help]),
            ...Object.values(CHOICES).flatMap((cs) => cs.map((c) => c.label)),
            ...Object.values(COMING_LATER).flatMap((c) => [c.label, c.now ?? ""]),
            ...Object.values(COMING_LATER_CHOICE),
            ...NAME_FIELDS.map((f) => f.label),
            ...SECTIONS.map((s) => s.title),
        ];
        expect(shown.map(leaks).filter((l) => l !== null)).toEqual([]);
    });

    it("rewrites a server sentence into the page's words", () => {
        const sentences = [
            "ledgeline.toml is a symbolic link, so the app won't write to it.",
            "ledgeline.toml [invoicer]: `filename` must contain {invoice_num}",
            "The invoicer settings in ledgeline.toml have a problem, so they can't be saved here.",
            "`company` names [[Nobody]], which isn't an organization note",
            "`words.total` can't be empty",
        ].map(settingsMessage);
        expect(sentences).toEqual([
            "The settings file is a symbolic link, so the app won't write to it.",
            "The settings file: new invoice file name must contain the number",
            "The invoicer settings in the settings file have a problem, so they can't be saved here.",
            "Your company names Nobody, which isn't an organization note",
            "Total rows can't be empty",
        ]);
        expect(sentences.map(leaks).filter((l) => l !== null)).toEqual([]);
    });
});
