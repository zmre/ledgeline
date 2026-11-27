// The time dashboard mounted (plan 46 T-dash): the URL's filters become the
// summary requests (this period and the one before it), a filter change is
// a new URL, `[`/`]` step the period, and before setup or with no time
// the page points at Track.
import {fireEvent, render, screen, waitFor} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";

const nav = vi.hoisted(() => ({url: new URL("http://localhost/app/time"), goto: vi.fn()}));
vi.mock("$app/state", () => ({
    page: {
        get url() {
            return nav.url;
        },
    },
}));
vi.mock("$app/navigation", () => ({goto: nav.goto}));

const api = vi.hoisted(() => ({summary: vi.fn(), meta: vi.fn()}));
vi.mock("../api/summary.js", async (original) => ({...(await original<object>()), fetchSummary: api.summary}));
vi.mock("./meta.js", async (original) => ({...(await original<object>()), fetchDashMeta: api.meta}));

import {listenForKeys, press} from "#lib/testing/keys.js";
import type {SummaryParams} from "../api/summary.js";
import {META, NOVEMBER, NOVEMBER_NOTES, OCTOBER, OCTOBER_NOTES} from "./fixtures.js";
import TimeDashboard from "./TimeDashboard.svelte";

const props = {entriesBase: "/app/time/entries", trackHref: "/app/time/track", today: "2026-11-26"};

function answer(params: SummaryParams) {
    const notes = params.group.includes("note");
    const november = params.from === "2026-11-01";
    return Promise.resolve(notes ? (november ? NOVEMBER_NOTES : OCTOBER_NOTES) : november ? NOVEMBER : OCTOBER);
}

describe("COMPONENT TimeDashboard", () => {
    listenForKeys();
    beforeEach(() => {
        nav.url = new URL("http://localhost/app/time");
        nav.goto.mockReset();
        api.summary.mockReset().mockImplementation(answer);
        api.meta.mockReset().mockResolvedValue(META);
    });

    it("asks for this month and last month to the same day, the fact table and the notes for each", async () => {
        render(TimeDashboard, props);
        await waitFor(() => expect(screen.getByTestId("tile-total-value").textContent).toBe("22h 54m"));
        const asked = api.summary.mock.calls.map((call) => call[0] as SummaryParams).map((p) => `${p.from}..${p.to} ${p.group.join(",")}`);
        expect(asked.sort()).toEqual([
            "2026-10-01..2026-10-26 account,customer,person,billable,billed,day",
            "2026-10-01..2026-10-26 note",
            "2026-11-01..2026-11-30 account,customer,person,billable,billed,day",
            "2026-11-01..2026-11-30 note",
        ]);
        expect(screen.getByTestId("time-dash-range").textContent).toContain("Nov 1 – 26, 2026 so far");
        expect(screen.getByTestId("tile-total-delta").textContent).toContain("+13h 54m");
    });

    it("passes the URL's filters on, and asks once with comparison off", async () => {
        nav.url = new URL("http://localhost/app/time?period=last-month&person=-&customer=Globex&billable=no&account=clients&compare=off");
        render(TimeDashboard, props);
        await waitFor(() => expect(api.summary).toHaveBeenCalledTimes(2));
        expect(api.summary).toHaveBeenCalledWith(
            expect.objectContaining({from: "2026-10-01", to: "2026-10-31", person: "", customer: "Globex", billable: false, account: "clients"})
        );
        await waitFor(() => expect(screen.getByTestId("tile-total-value")).toBeTruthy());
        expect(screen.queryByTestId("tile-total-delta")).toBeNull();
    });

    it("writes a filter into the URL, keeping the others", async () => {
        nav.url = new URL("http://localhost/app/time?period=ytd");
        render(TimeDashboard, props);
        await waitFor(() => screen.getByTestId("time-dash-billable"));
        await fireEvent.change(screen.getByTestId("time-dash-billable"), {target: {value: "yes"}});
        expect(nav.goto).toHaveBeenLastCalledWith("/app/time?period=ytd&billable=yes", {replace: false, reset: false});
        await fireEvent.change(screen.getByTestId("time-dash-person"), {target: {value: "sam"}});
        expect(nav.goto).toHaveBeenLastCalledWith("/app/time?period=ytd&person=sam", {replace: false, reset: false});
    });

    it("steps the period with [ and ] and the arrows beside it", async () => {
        render(TimeDashboard, props);
        await waitFor(() => screen.getByTestId("time-dash-period"));
        await press("[");
        expect(nav.goto).toHaveBeenLastCalledWith("/app/time?period=last-month", expect.anything());
        await press("]");
        expect(nav.goto).toHaveBeenLastCalledWith("/app/time?from=2026-12-01&to=2026-12-31", expect.anything());
        await fireEvent.click(screen.getByTestId("time-dash-prev"));
        expect(nav.goto).toHaveBeenLastCalledWith("/app/time?period=last-month", expect.anything());
    });

    it("offers custom dates when Custom is picked", async () => {
        render(TimeDashboard, props);
        await waitFor(() => screen.getByTestId("time-dash-period"));
        await fireEvent.change(screen.getByTestId("time-dash-period"), {target: {value: "custom"}});
        expect(nav.goto).toHaveBeenLastCalledWith("/app/time?from=2026-11-01&to=2026-11-30", expect.anything());
    });

    it("points at Track before setup, without asking for figures", async () => {
        api.meta.mockResolvedValue({...META, configured: false, anyTime: false});
        render(TimeDashboard, props);
        const setup = await screen.findByTestId("time-dash-setup");
        expect(setup.querySelector("a")?.getAttribute("href")).toBe("/app/time/track");
        expect(api.summary).not.toHaveBeenCalled();
    });

    it("points at Track when there is no time at all", async () => {
        api.meta.mockResolvedValue({...META, anyTime: false});
        render(TimeDashboard, props);
        expect((await screen.findByTestId("time-dash-empty")).textContent).toContain("No time yet");
    });

    it("says what failed and retries", async () => {
        api.summary.mockRejectedValueOnce(new Error("the books are gone"));
        render(TimeDashboard, props);
        expect((await screen.findByTestId("time-dash-error")).textContent).toContain("the books are gone");
    });
});
