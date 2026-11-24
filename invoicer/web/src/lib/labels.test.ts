// Friendly field names (Phase 4 feedback J10).
import {describe, expect, it} from "vitest";
import {fieldLabel, fieldList, friendlyMessage} from "./labels.js";

describe("UNIT labels", () => {
    it("names a field by its label, else its key in words", () => {
        expect(fieldLabel("invoicee_email")).toBe("Invoicee email");
        expect(fieldLabel("revenue_account")).toBe("Revenue account");
        expect(fieldLabel("po_number")).toBe("Po number");
        expect(fieldList(["terms", "phones", "ledger_customer"])).toBe("Terms, Phones, Name in the books");
    });

    it("rewrites keys in a server message, leaving other words", () => {
        expect(friendlyMessage("`invoicee_email` is missing")).toBe("Invoicee email is missing");
        expect(friendlyMessage("revenue_account: can't hold two spaces")).toBe("Revenue account: can't hold two spaces");
        expect(friendlyMessage("the name can't be empty")).toBe("The name can't be empty");
        expect(friendlyMessage("`unknown_key` is odd")).toBe("`unknown_key` is odd");
        expect(friendlyMessage("customers/Contoso.md changed on disk")).toBe("customers/Contoso.md changed on disk");
    });
});
