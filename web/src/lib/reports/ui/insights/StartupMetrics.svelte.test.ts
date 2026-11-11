// The startup-metrics section, mounted (plans/37). The figures come from the
// engine and `startup.ts` (node-tested); what only a mount can answer is which
// boxes, notes and disclosures reach the screen for a given report, and that
// the bridge's customer links point at the journal.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import type {AmountStyle} from "#lib/domain/types.js";
import type {SaasAvailable, SaasReport} from "#lib/reports/saasTypes.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {expectEveryBoxHelped, expectHelp} from "#lib/testing/helpTips.js";
import {billingReport, emptyPeriod, STARTUP_CURRENT, startupReport} from "#lib/testing/saasFixture.js";
import {METRIC_HELP, type MetricId} from "./metricHelp";
import StartupMetrics from "./StartupMetrics.svelte";

vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]};
const styles = new Map([["$", STYLE]]);

const mount = (report: SaasReport) => render(StartupMetrics, {report, styles});

const BOXES = ["arr", "netnewarr", "customers", "nrr", "burnmultiple", "grossmargin"] as const;

beforeEach(() => {
    settings.startupMoreOpen = false;
});

describe("COMPONENT StartupMetrics — headline boxes", () => {
    it("renders every box with the fixture's figures", () => {
        mount(startupReport());
        expect(screen.getByTestId("insights-box-arr-big").textContent).toBe("$70,200.00");
        expect(screen.getByTestId("insights-box-arr").textContent).toContain("ARR (2026-06 × 12)");
        expect(screen.getByTestId("insights-box-arr").textContent).toContain("MRR $5,850.00 · usage $100.00");
        expect(screen.getByTestId("insights-box-arr-small").textContent).toContain("$27,000.00 (62.5%)");
        expect(screen.getByTestId("insights-box-netnewarr-big").textContent).toBe("$27,000.00");
        expect(screen.getByTestId("insights-box-netnewarr-small").textContent).toContain("CMGR 4.1%/mo");
        expect(screen.getByTestId("insights-box-customers-big").textContent).toBe("8");
        expect(screen.getByTestId("insights-box-customers").textContent).toContain("1 new · 3 churned");
        expect(screen.getByTestId("insights-box-customers-small").textContent).toContain("0.0% growth");
        expect(screen.getByTestId("insights-box-nrr-big").textContent).toBe("120.8%");
        expect(screen.getByTestId("insights-box-nrr").textContent).toContain("trailing 12 months");
        expect(screen.getByTestId("insights-box-nrr-small").textContent).toContain("GRR 90.3%");
        expect(screen.getByTestId("insights-box-burnmultiple-big").textContent).toBe("1.6×");
        expect(screen.getByTestId("insights-box-burnmultiple").textContent).toContain("Good");
        expect(screen.getByTestId("insights-box-burnmultiple-small").textContent).toBe("net burn $44,070.00 (adjusted for prepayments)");
        expect(screen.getByTestId("insights-box-grossmargin-big").textContent).toBe("81.9%");
        expect(screen.getByTestId("insights-box-grossmargin-small").textContent).toContain("1.9 pp");
    });

    it("leaves each box out when its value is absent", () => {
        mount(startupReport({current: emptyPeriod({month: "2026-06", months: 12})}));
        for (const box of BOXES) expect(screen.queryByTestId(`insights-box-${box}`)).toBeNull();
        expect(screen.queryByTestId("startup-bridge")).toBeNull();
        expect(screen.queryByTestId("startup-more")).toBeNull();
    });

    it("colours the burn multiple by Sacks's band", () => {
        mount(startupReport({current: {...STARTUP_CURRENT, burnMultiple: 3.4}}));
        expect(screen.getByTestId("insights-box-burnmultiple-big").className).toContain("text-error");
        expect(screen.getByTestId("insights-box-burnmultiple").textContent).toContain("Bad");
    });
});

describe("COMPONENT StartupMetrics — hidden-customers note, one case per reason", () => {
    const hidden = (over: Partial<SaasAvailable>) =>
        startupReport({available: {...startupReport().available, customers: false, ...over}, current: {...STARTUP_CURRENT, customers: null, bridge: null}});

    it("low coverage", () => {
        mount(hidden({customersHiddenReason: "low-coverage", coveragePct: 62.4}));
        const note = screen.getByTestId("startup-customers-hidden");
        expect(note.textContent?.replace(/\s+/g, " ")).toContain("Customer metrics hidden: 62% of recurring revenue has a customer: tag (90% needed).");
        expect(note.querySelector("code")?.textContent).toBe("customer:");
    });

    it("no customer tags", () => {
        mount(hidden({customersHiddenReason: "no-customer-tags", coveragePct: 0}));
        expect(screen.getByTestId("startup-customers-hidden").textContent).toContain("no recurring revenue has a customer: tag.");
    });

    it("billing-clearing links to the deferred-revenue books section of the docs", () => {
        mount(hidden({customersHiddenReason: "billing-clearing"}));
        const note = screen.getByTestId("startup-customers-hidden");
        const link = within(note).getByRole("link", {name: "Deferred revenue books"});
        expect(link.getAttribute("href")).toBe("https://zmre.github.io/ledgeline/reports/startup-metrics/#deferred-revenue-books");
        expect(note.textContent?.replace(/\s+/g, " ")).toContain("Customer metrics hidden: your customer: tags are on invoices,");
        expect(note.textContent).toContain("Tag the recognition lines to turn these on.");
        expect(note.querySelector("code")?.textContent).toBe("customer:");
    });

    it("no note when customer metrics show", () => {
        mount(startupReport());
        expect(screen.queryByTestId("startup-customers-hidden")).toBeNull();
    });
});

describe("COMPONENT StartupMetrics — footnotes", () => {
    it("marks inferred customers with an asterisk and a footnote", () => {
        mount(startupReport({available: {...startupReport().available, inferredCustomers: true}}));
        expect(screen.getByTestId("insights-box-customers").textContent).toContain("Customers*");
        expect(screen.getByTestId("startup-inferred-note").textContent).toContain("Some customers inferred from payees");
    });

    it("has no inferred footnote otherwise", () => {
        mount(startupReport());
        expect(screen.queryByTestId("startup-inferred-note")).toBeNull();
    });
});

describe("COMPONENT StartupMetrics — segment table", () => {
    it("opens from a box's chevron with a row per segment, and switches keys", async () => {
        mount(startupReport());
        expect(screen.queryByTestId("startup-segments")).toBeNull();
        const toggle = screen.getByTestId("insights-box-arr-segments-toggle");
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
        await fireEvent.click(toggle);
        const rows = screen.getAllByTestId("startup-segments-row");
        expect(rows.map((row) => row.querySelector("td")?.textContent)).toEqual(["mid", "enterprise", "smb"]);
        expect(rows[0].textContent).toContain("$2,900.00");
        expect(rows[2].textContent).toContain("50.0%");

        await fireEvent.change(screen.getByTestId("startup-segments-key"), {target: {value: "product"}});
        expect(screen.getAllByTestId("startup-segments-row").map((row) => row.querySelector("td")?.textContent)).toEqual(["core", "platform"]);

        // Any of the three chevrons closes it again.
        await fireEvent.click(screen.getByTestId("insights-box-nrr-segments-toggle"));
        expect(screen.queryByTestId("startup-segments")).toBeNull();
    });

    it("offers no chevron without dimensions", () => {
        mount(startupReport({segments: []}));
        expect(screen.queryByTestId("insights-box-arr-segments-toggle")).toBeNull();
    });
});

describe("COMPONENT StartupMetrics — MRR bridge", () => {
    it("lists start, each category, and end", () => {
        mount(startupReport());
        expect(screen.getByTestId("startup-bridge-start").textContent).toContain("$3,600.00");
        expect(screen.getByTestId("startup-bridge-churn").textContent).toContain("Churn(3)");
        expect(screen.getByTestId("startup-bridge-churn").textContent).toContain("−$800.00");
        expect(screen.queryByTestId("startup-bridge-other")).toBeNull();
        expect(screen.getByTestId("startup-bridge-end").textContent).toContain("$5,850.00");
    });

    it("expands a category to its customers, each linking to the journal searching for them", async () => {
        mount(startupReport());
        // The row's toggle, not its (i): only the toggle has `aria-expanded`.
        const churn = within(screen.getByTestId("startup-bridge-churn")).getByRole("button", {expanded: false});
        await fireEvent.click(churn);
        expect(churn.getAttribute("aria-expanded")).toBe("true");
        const links = within(screen.getByTestId("startup-bridge-churn-customers")).getAllByRole("link");
        expect(links.map((link) => link.textContent?.replace(/\s+/g, " ").trim())).toEqual([
            "Fjord Studio smb $350.00",
            "Lumen Labs smb $250.00",
            "Pinecrest Dental smb $200.00",
        ]);
        const href = new URL(links[0].getAttribute("href") ?? "", "http://localhost");
        expect(href.pathname).toBe("/");
        expect(href.searchParams.get("q")).toBe("Fjord Studio");

        await fireEvent.click(churn);
        expect(screen.queryByTestId("startup-bridge-churn-customers")).toBeNull();
    });

    it("shows Other with its explanation when non-zero", () => {
        const current = {...STARTUP_CURRENT, bridge: {...STARTUP_CURRENT.bridge!, other: {m: -15000n, p: 2}}};
        mount(startupReport({current}));
        const other = screen.getByTestId("startup-bridge-other");
        expect(other.textContent).toContain("Other");
        expectHelp(other, METRIC_HELP["bridge-other"]);
    });
});

describe("COMPONENT StartupMetrics — More metrics", () => {
    it("is collapsed by default and remembers being opened", async () => {
        mount(startupReport());
        expect(screen.queryByTestId("startup-more-arpa")).toBeNull();
        await fireEvent.click(screen.getByRole("checkbox", {name: "Toggle More metrics"}));
        expect(screen.getByTestId("startup-more-arpa").textContent).toBe("$731.25");
        expect(settings.startupMoreOpen).toBe(true);
    });
});

describe("COMPONENT StartupMetrics — (i) help everywhere", () => {
    it("every box has its help", () => {
        mount(startupReport());
        expect(expectEveryBoxHelped().sort()).toEqual([...BOXES].sort());
    });

    it("every More metrics row has its help", () => {
        settings.startupMoreOpen = true;
        mount(startupReport());
        const values = [...document.querySelectorAll<HTMLElement>('[data-testid^="startup-more-"]')];
        expect(values.length).toBeGreaterThan(5);
        for (const value of values) {
            const key = (value.dataset.testid ?? "").slice("startup-more-".length) as MetricId;
            const dt = value.parentElement?.querySelector("dt");
            expect(dt, key).toBeTruthy();
            expectHelp(dt!, METRIC_HELP[key]);
        }
    });

    it("every bridge line has its help, including Other", () => {
        const current = {...STARTUP_CURRENT, bridge: {...STARTUP_CURRENT.bridge!, other: {m: -15000n, p: 2}}};
        mount(startupReport({current}));
        for (const category of ["new", "expansion", "contraction", "churn", "reactivation", "other"] as const) {
            expectHelp(screen.getByTestId(`startup-bridge-${category}`), METRIC_HELP[`bridge-${category}`]);
        }
    });

    it("the segment table's ARPA and NRR headers have their help", async () => {
        mount(startupReport());
        await fireEvent.click(screen.getByTestId("insights-box-arr-segments-toggle"));
        const headers = within(screen.getByTestId("startup-segments")).getAllByRole("columnheader");
        const arpa = headers.find((th) => th.textContent?.trim().startsWith("ARPA"));
        const nrr = headers.find((th) => th.textContent?.trim().startsWith("NRR"));
        expectHelp(arpa!, METRIC_HELP["segment-arpa"]);
        expectHelp(nrr!, METRIC_HELP["segment-nrr"]);
    });
});

describe("COMPONENT StartupMetrics — customers from billings (Amendment A1.2)", () => {
    it("shows the billing count in the Customers box, with its own help", () => {
        mount(billingReport());
        const box = screen.getByTestId("insights-box-customers");
        expect(box.textContent).toContain("Customers (from billings)");
        expect(screen.getByTestId("insights-box-customers-big").textContent).toBe("12");
        expect(box.textContent).toContain("3 new · 1 lapsed");
        expect(screen.getByTestId("insights-box-customers-small").textContent).toContain("20.0% growth");
        expectHelp(box, METRIC_HELP["customers-billings"]);
        // Still hidden: everything per-customer MRR feeds.
        expect(screen.queryByTestId("insights-box-nrr")).toBeNull();
        expect(screen.queryByTestId("startup-bridge")).toBeNull();
        expect(screen.queryByTestId("insights-box-customers-segments-toggle")).toBeNull();
    });

    it("says where the count comes from in the hidden note", () => {
        mount(billingReport());
        const note = screen.getByTestId("startup-customers-hidden").textContent?.replace(/\s+/g, " ") ?? "";
        expect(note).toMatch(/^ ?Per-customer MRR figures hidden: your customer: tags are on invoices/);
        expect(note).toContain("Customer counts are from billings.");
    });

    it("footnotes a short window, and asterisks inferred customers", () => {
        mount(billingReport({windowShort: true, inferred: true}));
        expect(screen.getByTestId("startup-window-short-note").textContent).toBe("Counts may be low until there are 12 months of history.");
        expect(screen.getByTestId("insights-box-customers").textContent).toContain("Customers (from billings)*");
        expect(screen.getByTestId("startup-inferred-note")).toBeTruthy();
    });

    it("has no window footnote with a full year of history", () => {
        mount(billingReport());
        expect(screen.queryByTestId("startup-window-short-note")).toBeNull();
    });

    it("lists ACV and logo churn from billings under More metrics", async () => {
        mount(billingReport());
        await fireEvent.click(screen.getByRole("checkbox", {name: "Toggle More metrics"}));
        expect(screen.getByTestId("startup-more-acv-billings").textContent).toBe("$12,600.00");
        expect(screen.getByTestId("startup-more-logo-churn-billings").textContent).toBe("10.0%");
    });
});

describe("COMPONENT StartupMetrics — closed through (A2)", () => {
    it("says where the metrics stop when the books aren't closed to the span's end, with help", () => {
        mount(startupReport({period: {...startupReport().period, end: "2026-10-31", currEnd: "2026-10-31"}}));
        const tag = screen.getByTestId("startup-closed-through");
        expect(tag.textContent).toContain("Closed through 2026-06");
        expectHelp(tag, METRIC_HELP["closed-through"]);
    });

    it("says nothing when the books are closed through the span's last month", () => {
        mount(startupReport());
        expect(screen.queryByTestId("startup-closed-through")).toBeNull();
    });
});
