// Recently updated (R15), mounted on the server's golden: a card per
// invoice, latest first, each a link to it, its status in words on a band
// of the list badges' colours.
import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import RecentInvoices from "./RecentInvoices.svelte";
import {golden} from "../dashboardGolden.js";
import {listenForKeys, press} from "#lib/testing/keys.js";

const invoiceHref = (number: string) => `/app/invoices/${number}`;

describe("COMPONENT RecentInvoices", () => {
    it("shows the latest first, each card a link to its invoice", () => {
        render(RecentInvoices, {recent: golden("accrual").recent, invoiceHref});
        const cards = screen.getAllByTestId("recent-card");
        expect(cards.map((c) => c.dataset.number)).toEqual(["106", "108", "105", "107", "102", "103"]);
        const first = within(cards[0]).getByRole("link");
        expect(first.getAttribute("href")).toBe("/app/invoices/106");
        expect(first.textContent).toContain("No. 106");
        expect(first.textContent).toContain("Wingtip");
        expect(first.textContent).toContain("Edited 2026-11-21");
        expect(first.textContent).toContain("$1,200.00");
    });

    it("bands each card with its status, in words and the list's colours", () => {
        render(RecentInvoices, {recent: golden("accrual").recent, invoiceHref});
        const bands = screen.getAllByTestId("recent-status");
        expect(bands.map((b) => b.textContent)).toEqual(["Overdue", "Draft", "Open", "Overdue", "Paid", "Void"]);
        expect(bands[0].className).toContain("border-error");
        expect(bands[1].className).toContain("border-base-content/30");
        expect(bands[2].className).toContain("border-warning");
        expect(bands[4].className).toContain("border-success");
    });

    it("scrolls sideways when narrow and becomes a grid when wide", () => {
        render(RecentInvoices, {recent: golden("cash").recent, invoiceHref});
        const row = screen.getByTestId("recent-cards");
        for (const cls of ["overflow-x-auto", "snap-x", "@3xl:grid", "@5xl:grid-cols-6"]) expect(row.className).toContain(cls);
    });

    it("shows a card without a number, unlinked", () => {
        const [first] = golden("cash").recent;
        render(RecentInvoices, {recent: [{...first, number: null}], invoiceHref});
        const card = screen.getByTestId("recent-card");
        expect(within(card).queryByRole("link")).toBeNull();
        expect(card.textContent).toContain("No number");
    });
});

describe("COMPONENT RecentInvoices keys (PLAN 5k)", () => {
    listenForKeys();

    it("steps focus through the cards with j and k", async () => {
        render(RecentInvoices, {recent: golden("accrual").recent, invoiceHref});
        const links = screen.getAllByRole("link");
        await press("j");
        expect(document.activeElement).toBe(links[0]);
        await press("j");
        expect(document.activeElement).toBe(links[1]);
        await press("k");
        await press("k");
        expect(document.activeElement).toBe(links[0]);
    });
});
