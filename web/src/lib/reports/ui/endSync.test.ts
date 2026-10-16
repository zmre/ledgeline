import {beforeEach, describe, expect, it} from "vitest";
import {reportsEnd} from "#lib/stores/reportsEnd.js";
import {createEndSync} from "./endSync";
import {defaultReportParams, type ReportParams} from "./params";

const DFLT = defaultReportParams("2026-07-08");

/** Drive a sync the way the page does: apply each answer to the params. */
function page(search = "", over: Partial<ReportParams> = {}) {
    const params: ReportParams = {...DFLT, ...over};
    const sync = createEndSync(reportsEnd);
    Object.assign(params, sync.restore(params, search));
    const settle = () => Object.assign(params, sync.observe(params));
    settle();
    return {
        params,
        edit(change: Partial<ReportParams>) {
            Object.assign(params, change);
            settle();
        },
    };
}

describe("UNIT reports/ui/endSync", () => {
    beforeEach(() => reportsEnd.reset());

    it("remembers nothing from the defaults alone", () => {
        const p = page();
        p.edit({tab: "bs"});
        p.edit({tab: "is"});
        expect(reportsEnd.get()).toBeNull();
        expect(p.params.asOf).toBe("2026-07-08");
        expect(p.params.from).toBe("2026-01-01");
        expect(p.params.to).toBe("2026-12-31");
    });

    it("carries an end set on one tab to the next tab opened", () => {
        const p = page("", {tab: "bs"});
        p.edit({asOf: "2026-03-31"});
        expect(reportsEnd.get()).toBe("2026-03-31");

        p.edit({tab: "is"});
        expect(p.params.from).toBe("2026-01-01");
        expect(p.params.to).toBe("2026-03-31");

        p.edit({tab: "cf"});
        expect(p.params.cfFrom).toBe("2025-04-01");
        expect(p.params.cfTo).toBe("2026-03-31");

        p.edit({tab: "nw"});
        expect(p.params.end).toBe("2026-03-31");
    });

    it("remembers the MOST RECENT end, whichever tab set it", () => {
        const p = page("", {tab: "bs"});
        p.edit({asOf: "2026-03-31"});
        p.edit({tab: "is"});
        p.edit({to: "2026-06-30"});
        p.edit({tab: "bs"});
        expect(p.params.asOf).toBe("2026-06-30");
    });

    it("passes through a tab with no end without forgetting", () => {
        const p = page("", {tab: "bs"});
        p.edit({asOf: "2026-03-31"});
        p.edit({tab: "subs"});
        p.edit({tab: "nw"});
        expect(p.params.end).toBe("2026-03-31");
    });

    it("survives leaving the page: a new page opens on the remembered end", () => {
        page("", {tab: "bs"}).edit({asOf: "2026-03-31"});
        const again = page("", {tab: "is"});
        expect(again.params.from).toBe("2026-01-01");
        expect(again.params.to).toBe("2026-03-31");
    });

    it("lets a URL end win on load, and remembers it", () => {
        reportsEnd.set("2026-03-31");
        const p = page("?tab=bs&asof=2025-12-31", {tab: "bs", asOf: "2025-12-31"});
        expect(p.params.asOf).toBe("2025-12-31");
        expect(reportsEnd.get()).toBe("2025-12-31");
        p.edit({tab: "is"});
        expect(p.params.from).toBe("2025-01-01");
        expect(p.params.to).toBe("2025-12-31");
    });

    it("applies the remembered end when the URL names no end", () => {
        reportsEnd.set("2026-03-31");
        const p = page("?tab=bs", {tab: "bs"});
        expect(p.params.asOf).toBe("2026-03-31");
    });
});
