// The Send panel's wiring (SendSection), mounted with the API mocked: right
// after a send, until the delivery is fetched again, Export must not act on
// the draft's state (it would ask "Mark as sent?" of a sent invoice), by a
// click or by Shift+P.
import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {describe, expect, it, vi} from "vitest";
import type {Delivery} from "#lib/delivery.js";
import {listenForKeys, press} from "#lib/testing/keys.js";

const api = vi.hoisted(() => ({fetchDelivery: vi.fn(), exportPdf: vi.fn()}));
vi.mock("#lib/api.js", async (original) => ({...(await original<object>()), fetchDelivery: api.fetchDelivery, exportPdf: api.exportPdf}));

import SendSection from "./SendSection.svelte";
import {DELIVERY_108} from "./fixtures.js";

const SENT: Delivery = {...DELIVERY_108, status: "sent", sent_date: "2026-11-17", sent_via: "email"};

describe("COMPONENT SendSection", () => {
    listenForKeys();

    it("holds Export while the delivery is fetched again after a send", async () => {
        let answer: (d: Delivery) => void = () => {};
        api.fetchDelivery.mockReset().mockResolvedValueOnce(DELIVERY_108);
        api.exportPdf.mockReset().mockResolvedValue({destination: "kept", file: "x.pdf"});
        const view = render(SendSection, {number: "108", reload: 0});
        const button = await screen.findByTestId("send-export");
        expect((button as HTMLButtonElement).disabled).toBe(false);

        // The page bumps `reload` after the send; the new delivery is still on its way.
        api.fetchDelivery.mockReturnValueOnce(new Promise<Delivery>((resolve) => (answer = resolve)));
        await view.rerender({number: "108", reload: 1});
        await tick();
        // Disabled (a browser fires no click on it; jsdom's fireEvent would), and P is off.
        expect((screen.getByTestId("send-export") as HTMLButtonElement).disabled).toBe(true);
        await press("P");
        expect(screen.queryByTestId("send-prompt")).toBeNull();
        expect(api.exportPdf).not.toHaveBeenCalled();

        // The sent delivery lands: Export exports at once, no prompt.
        answer(SENT);
        await vi.waitFor(() => expect(screen.getByTestId("send-export").textContent?.trim()).toBe("Export PDF"));
        await press("P");
        expect(screen.queryByTestId("send-prompt")).toBeNull();
        await vi.waitFor(() => expect(api.exportPdf).toHaveBeenCalledWith("108", {temp: false}));
    });

    it("shows the newest delivery when an older fetch lands after it", async () => {
        let older: (d: Delivery) => void = () => {};
        let newer: (d: Delivery) => void = () => {};
        api.fetchDelivery
            .mockReset()
            .mockReturnValueOnce(new Promise<Delivery>((resolve) => (older = resolve)))
            .mockReturnValueOnce(new Promise<Delivery>((resolve) => (newer = resolve)));
        const view = render(SendSection, {number: "108", reload: 0});
        await view.rerender({number: "108", reload: 1});
        await tick();
        expect(api.fetchDelivery).toHaveBeenCalledTimes(2);

        newer(SENT);
        await vi.waitFor(() => expect(screen.getByTestId("send-sent")).toBeTruthy());
        older(DELIVERY_108);
        await new Promise((resolve) => setTimeout(resolve, 0));
        await tick();
        expect(screen.queryByTestId("send-sent")).not.toBeNull();
    });
});
