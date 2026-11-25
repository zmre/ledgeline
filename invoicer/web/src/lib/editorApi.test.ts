// The editor's API calls that shape what they answer.
import {afterEach, describe, expect, it, vi} from "vitest";
import {applyPayment, createInvoice} from "./editorApi.js";

const answer = (body: unknown, status = 200) => vi.fn(async () => new Response(JSON.stringify(body), {status}));

describe("UNIT editorApi", () => {
    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("answers New's warnings (the draft is written either way), [] when there are none", async () => {
        const made = {path: "invoices/x.md", number: "2005", total: null, written: true, copied: null};
        vi.stubGlobal("fetch", answer({...made, warnings: ["the draft is written, but the Modern template wasn't copied in: denied"]}));
        expect((await createInvoice({customer: "[[Customer X]]", date: "2026-11-17", deposit: true})).warnings).toEqual([
            "the draft is written, but the Modern template wasn't copied in: denied",
        ]);
        vi.stubGlobal("fetch", answer(made));
        expect((await createInvoice({customer: "[[Customer X]]", date: "2026-11-17"})).warnings).toEqual([]);
    });

    it("sends New's deposit flag", async () => {
        const fetch = answer({path: "invoices/x.md", number: "2005", total: null, written: true, warnings: []});
        vi.stubGlobal("fetch", fetch);
        await createInvoice({customer: "[[Customer X]]", date: "2026-11-17", deposit: true});
        const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(JSON.parse(String(init.body))).toMatchObject({deposit: true});
    });

    it("previews an apply with dry_run, and refuses with the server's code", async () => {
        const fetch = answer({written: false, applied: []});
        vi.stubGlobal("fetch", fetch);
        await applyPayment("207", "0123456789abcdef", {dryRun: true});
        const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(url).toBe("/api/invoices/207/payments");
        expect(JSON.parse(String(init.body))).toMatchObject({reference: "0123456789abcdef", dry_run: true});
        vi.stubGlobal("fetch", answer({error: "these are accrual books", code: "refused"}, 422));
        await expect(applyPayment("105", "0123456789abcdef")).rejects.toMatchObject({code: "refused", status: 422});
    });
});
