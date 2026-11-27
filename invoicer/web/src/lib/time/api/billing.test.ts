// Billing's calls: each sends what `app/time.rs` reads, previews say so, and
// an answer that isn't the expected shape is refused.
import {afterEach, describe, expect, it, vi} from "vitest";
import {billCreated, billPreview, payCreated, payPreview, riley, unbilled} from "../billing/fixtures.js";
import {createBill, createPay, fetchUnbilled, fetchUnpaid, isBooked, previewBill, previewPay, releaseBill, releaseTime} from "./billing.js";

function answer(body: unknown) {
    const fetch = vi.fn<(url: string, init?: RequestInit) => Promise<Response>>(
        async () => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}})
    );
    vi.stubGlobal("fetch", fetch);
    return fetch;
}

const sent = (fetch: ReturnType<typeof answer>) => {
    const [url, init] = fetch.mock.calls[0];
    return {url, method: init?.method, body: init?.body === undefined ? undefined : JSON.parse(String(init.body))};
};

describe("UNIT time billing API", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("reads Unbilled and To pay", async () => {
        let fetch = answer(unbilled());
        expect((await fetchUnbilled()).customers[0].customer).toBe("Acme Corp");
        expect(sent(fetch)).toMatchObject({url: "/api/time/unbilled", method: "GET"});
        fetch = answer([riley()]);
        expect((await fetchUnpaid())[0].person).toBe("riley");
        expect(sent(fetch).url).toBe("/api/time/unpaid");
        answer({customers: "no"});
        await expect(fetchUnbilled()).rejects.toThrow(/shape/);
    });

    it("previews a bill, then creates it with the same request", async () => {
        let fetch = answer(billPreview());
        await previewBill({customer: "Acme Corp", from: "2026-10-01", to: "2026-10-31"});
        expect(sent(fetch)).toMatchObject({url: "/api/time/bill", method: "POST", body: {customer: "Acme Corp", from: "2026-10-01", preview: true}});
        fetch = answer(billCreated());
        expect((await createBill({customer: "Acme Corp"})).number).toBe("102");
        expect(sent(fetch).body).toEqual({customer: "Acme Corp"});
    });

    it("releases an invoice's time, or voids a bill and releases its", async () => {
        const fetch = answer({done: 2, files: []});
        await releaseTime("104");
        expect(sent(fetch)).toMatchObject({url: "/api/time/release", body: {invoice: "104", how: "release"}});
        const bill = answer({done: 1, files: []});
        await releaseBill("sam timesheet 2026-11");
        expect(sent(bill)).toMatchObject({url: "/api/time/release", body: {bill: "sam timesheet 2026-11", how: "release"}});
    });

    it("tells a booked bill from a preview that still needs something", async () => {
        const fetch = answer(payPreview());
        await previewPay({person: "riley"});
        expect(sent(fetch).body).toEqual({person: "riley", preview: true});
        const booking = answer(payCreated());
        const booked = await createPay({person: "riley", rate: "$60"});
        expect(sent(booking).body).toEqual({person: "riley", rate: "$60"});
        expect(isBooked(booked)).toBe(true);
        answer(payPreview());
        expect(isBooked(await createPay({person: "riley"}))).toBe(false);
    });
});
