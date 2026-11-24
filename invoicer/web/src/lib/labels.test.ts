import {describe, expect, it} from "vitest";
import {fieldLabel, friendlyProblem} from "./labels.js";

describe("friendlyProblem", () => {
    it("names keys by their labels", () => {
        expect(friendlyProblem("`invoicee_email` is missing")).toBe("Invoicee email is missing");
        expect(friendlyProblem("`due_date` 2026-10-01 is before `date` 2026-11-01")).toBe("Due date 2026-10-01 is before date 2026-11-01");
        expect(friendlyProblem("`po_num` is now `reference`")).toBe("PO number is now reference");
        expect(friendlyProblem("`deposit_for` must be text, not a list or map")).toBe("Deposit for note must be text, not a list or map");
    });

    it("quotes a key's value and any other code", () => {
        expect(friendlyProblem("`status: sending` isn't one of draft, sent, void")).toBe("Status “sending” isn't one of draft, sent, void");
        expect(friendlyProblem("`invoice_num` can't hold a comma: it is the journal's `invoice:` tag")).toBe(
            "Invoice number can't hold a comma: it is the journal's “invoice:” tag"
        );
        expect(friendlyProblem("`customer: Customer X` isn't a [[wikilink]]")).toBe("Customer “Customer X” isn't a [[wikilink]]");
    });

    it("leaves plain text alone, with a capital first letter", () => {
        expect(friendlyProblem("there is no bill table")).toBe("There is no bill table");
        expect(friendlyProblem("")).toBe("");
    });
});

describe("fieldLabel", () => {
    it("knows the invoicer's keys and spells out the rest", () => {
        expect(fieldLabel("invoicee_address")).toBe("Invoicee address");
        expect(fieldLabel("support_tier")).toBe("Support tier");
    });
});
