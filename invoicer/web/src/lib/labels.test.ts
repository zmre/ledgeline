// Friendly field names (Phase 4 feedback H1, J10).
import {describe, expect, it} from "vitest";
import {fieldLabel, fieldList, friendlyMessage} from "./labels.js";

describe("UNIT labels", () => {
    it("names a field by its label, else its key in words", () => {
        expect(fieldLabel("invoicee_email")).toBe("Invoicee email");
        expect(fieldLabel("revenue_account")).toBe("Revenue account");
        expect(fieldLabel("deposit_for")).toBe("Deposit for note");
        expect(fieldLabel("po_number")).toBe("Po number");
        expect(fieldList(["terms", "phones", "ledger_customer"])).toBe("Terms, Phones, Name in the books");
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
});
