import {describe, expect, it} from "vitest";
import {
    accountNameProblem,
    accountProblems,
    commodityProblem,
    createBody,
    CURRENCY_PRESETS,
    defaultTitle,
    effectiveTypeName,
    extensionOf,
    nextStep,
    previousStep,
    removeAccount,
    renameAccount,
    sampleAmount,
    SOURCE_APPS,
    sourceFileWarning,
    stepsFor,
    tagSummary,
    titleProblem,
} from "./model";
import type {TemplateAccount} from "./types";

const account = (name: string, tags: [string, string][] = []): TemplateAccount => ({name, tags: tags.map(([n, value]) => ({name: n, value}))});

const CHART: readonly TemplateAccount[] = [
    account("assets", [["type", "A"]]),
    account("assets:bank", [
        ["type", "C"],
        ["bsgroup", "Cash and cash equivalents"],
    ]),
    account("assets:bank:checking"),
    account("transfers", [
        ["type", "A"],
        ["clearing", ""],
    ]),
    account("misc"),
];

describe("UNIT onboarding model", () => {
    describe("steps", () => {
        it("visits the review only for a chart that has accounts", () => {
            expect(stepsFor(null, "individual")).toEqual(["start"]);
            expect(stepsFor("fresh", "business")).toEqual(["start", "kind", "details", "accounts", "create"]);
            expect(stepsFor("fresh", "empty")).toEqual(["start", "kind", "details", "create"]);
            expect(stepsFor("import", "individual")).toEqual(["start", "source", "details", "create"]);
        });

        it("moves forward and back along the run, stopping at the ends", () => {
            const steps = stepsFor("fresh", "individual");
            expect(nextStep(steps, "start")).toBe("kind");
            expect(nextStep(steps, "create")).toBeNull();
            expect(previousStep(steps, "accounts")).toBe("details");
            expect(previousStep(steps, "start")).toBeNull();
            // A step this run does not visit goes nowhere.
            expect(nextStep(steps, "source")).toBeNull();
        });
    });

    describe("currency", () => {
        it("writes one thousand in the chosen style", () => {
            const style = (id: string) => CURRENCY_PRESETS.find((preset) => preset.id === id)!.style;
            expect(sampleAmount(style("usd"))).toBe("$1,000.00");
            expect(sampleAmount(style("eur"))).toBe("1.000,00 €");
            expect(sampleAmount(style("jpy"))).toBe("¥1,000");
            expect(sampleAmount(style("chf"))).toBe("1,000.00 CHF");
        });

        it("accepts letters or one currency symbol, as the engine does", () => {
            for (const ok of ["$", "€", "USD", "C$", "kr"]) expect(commodityProblem(ok)).toBeNull();
            for (const bad of ["", "1USD", "U S", "$;", '"$"', "TOOLONGCODE", "-"]) expect(commodityProblem(bad)).not.toBeNull();
        });
    });

    describe("titles and names", () => {
        it("wants a one-line title of sensible length", () => {
            expect(titleProblem("  Acme Books ")).toBeNull();
            expect(titleProblem("   ")).not.toBeNull();
            expect(titleProblem("a\nb")).not.toBeNull();
            expect(titleProblem("x".repeat(121))).not.toBeNull();
        });

        it("names the folder's own name as the default title", () => {
            expect(defaultTitle("Acme Books", "business")).toBe("Acme Books");
            expect(defaultTitle(null, "business")).toBe("Business books");
            expect(defaultTitle(" ", "individual")).toBe("My books");
        });

        it("refuses account names the accounts editor refuses", () => {
            expect(accountNameProblem("assets:bank")).toBeNull();
            for (const bad of ["", " assets", "assets;x", "assets#x", "assets  bank", "a\tb\tc\n"]) expect(accountNameProblem(bad)).not.toBeNull();
        });

        it("flags bad names and repeats by index", () => {
            const problems = accountProblems([account("a"), account("b;"), account("a")]);
            expect([...problems.keys()]).toEqual([1, 2]);
            expect(problems.get(2)).toMatch(/already/);
        });
    });

    describe("the account list", () => {
        it("reads a type from the account or its nearest listed parent", () => {
            expect(effectiveTypeName(CHART, CHART[2])).toBe("Cash");
            expect(effectiveTypeName(CHART, CHART[0])).toBe("Asset");
            expect(effectiveTypeName(CHART, CHART[4])).toBeNull();
        });

        it("summarises the non-type tags as the file will spell them", () => {
            expect(tagSummary(CHART[1])).toBe("bsgroup: Cash and cash equivalents");
            expect(tagSummary(CHART[3])).toBe("clearing:");
            expect(tagSummary(CHART[2])).toBe("");
        });

        it("renames and removes without touching the original", () => {
            const renamed = renameAccount(CHART, 2, "assets:bank:current");
            expect(renamed[2].name).toBe("assets:bank:current");
            expect(CHART[2].name).toBe("assets:bank:checking");
            expect(removeAccount(CHART, 0).map((entry) => entry.name)).toEqual(CHART.slice(1).map((entry) => entry.name));
        });
    });

    describe("the export file", () => {
        const qif = SOURCE_APPS.find((app) => app.id === "banktivity")!;

        it("reads the extension", () => {
            expect(extensionOf("Books.QIF")).toBe("qif");
            expect(extensionOf("README")).toBe("");
            expect(extensionOf(".hidden")).toBe("");
        });

        it("warns about a file the app does not export, or Ledgeline can't read yet", () => {
            expect(sourceFileWarning(qif, "books.qif", ["qif", "csv"])).toBeNull();
            expect(sourceFileWarning(qif, "books.qif", null)).toBeNull();
            expect(sourceFileWarning(qif, "books.csv", ["csv"])).toMatch(/exports \.qif/);
            expect(sourceFileWarning(qif, "books.qif", ["csv", "ofx"])).toMatch(/can't import \.qif/);
        });
    });

    it("builds the create body with a trimmed title", () => {
        const body = createBody({folderId: "f1", title: "  Mine  ", currency: CURRENCY_PRESETS[0].style, accounts: CHART});
        expect(body).toEqual({folderId: "f1", title: "Mine", currency: CURRENCY_PRESETS[0].style, accounts: CHART});
    });
});
