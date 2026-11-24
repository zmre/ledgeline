import {describe, expect, it} from "vitest";
import {anchorOf, currentPdf, defaultVia, exportedText, exportPrompt, sendBlocker, sizeText} from "./delivery.js";
import {DELIVERY_105, DELIVERY_108} from "./components/fixtures.js";

describe("UNIT delivery", () => {
    it("asks a draft whether to mark it sent, and warns on a changed sent invoice", () => {
        expect(exportPrompt({status: "draft", changed_since_sent: null})).toBe("mark-sent");
        expect(exportPrompt({status: "sent", changed_since_sent: true})).toBe("changed");
        expect(exportPrompt({status: "sent", changed_since_sent: false})).toBe("none");
        expect(exportPrompt({status: "sent", changed_since_sent: null})).toBe("none");
        expect(exportPrompt({status: "void", changed_since_sent: true})).toBe("none");
    });

    it("defaults to the portal for a portal customer", () => {
        expect(defaultVia(DELIVERY_108)).toBe("portal");
        expect(defaultVia({...DELIVERY_108, portal: null})).toBe("email");
        expect(defaultVia({...DELIVERY_108, sent_via: "mail"})).toBe("mail");
    });

    it("blocks sending to a portal that needs a reference when there is none", () => {
        expect(sendBlocker(DELIVERY_108)).toBeNull();
        const missing = {...DELIVERY_108, copy: {...DELIVERY_108.copy, reference: null}};
        expect(sendBlocker(missing)).toContain("Coupa rejects invoices without a reference");
        expect(sendBlocker({...missing, portal: {...DELIVERY_108.portal!, requires_reference: false}})).toBeNull();
    });

    it("says where an export went", () => {
        expect(exportedText({destination: "kept", kept_at: "invoices/pdf/x.pdf", file: "x.pdf"})).toBe("Saved invoices/pdf/x.pdf");
        expect(exportedText({destination: "temp", file: "x.pdf"})).toContain("temporary");
        expect(exportedText({destination: "copy", file: "x.pdf"})).toBe("Saved a copy: x.pdf");
    });

    it("names one current PDF: this session's export, else the latest kept", () => {
        expect(currentPdf(DELIVERY_105, null)).toEqual({
            handle: {destination: "kept", file: "2026-11-10 Customer X Invoice 105.pdf"},
            text: "Kept invoices/pdf/2026-11-10 Customer X Invoice 105.pdf",
        });
        expect(currentPdf(DELIVERY_108, null)).toBeNull();
        const exported = {
            number: "108",
            document: DELIVERY_108.document,
            destination: "temp" as const,
            file: "x.pdf",
            bytes: 1,
            changed_since_sent: null,
            warnings: [],
        };
        expect(currentPdf(DELIVERY_108, exported)?.handle).toEqual({destination: "temp", file: "x.pdf"});
        expect(currentPdf(DELIVERY_108, exported)?.text).toContain("temporary");
    });

    it("sizes and anchors", () => {
        expect(sizeText(900)).toBe("900 B");
        expect(sizeText(48_000)).toBe("47 KB");
        expect(sizeText(2_500_000)).toBe("2.4 MB");
        expect(anchorOf({left: 1, top: 2, width: 3, height: 4})).toEqual({x: 1, y: 2, width: 3, height: 4});
        expect(DELIVERY_105.destination).toBe("kept");
    });
});
