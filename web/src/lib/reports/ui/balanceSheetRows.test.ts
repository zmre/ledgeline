// The grouped balance sheet's display model: what is on screen at a given
// collapse state, and how one MixedAmount becomes one figure plus footnotes.
//
// These are the two claims the view rests on and cannot make about itself:
// a collapsed group contributes exactly one row (so `j` has somewhere to go on
// first load, and the cursor never points at an invisible account), and an
// unpriced commodity is demoted rather than dropped.

import {describe, expect, it} from "vitest";
import {decodeBalanceSheetReport} from "$lib/api/nativeDecode";
import {dec, type MixedAmount} from "$lib/domain/money";
import type {AmountStyle} from "$lib/domain/types";
import type {BsSection} from "$lib/reports/types";
import {CLASSIFIED_BALANCE_SHEET, GROUPED_BALANCE_SHEET, STRADDLING_BALANCE_SHEET, UNBALANCED_BALANCE_SHEET} from "$lib/testing/balanceSheetFixture";
import {amountCell, bsCursorRows, bsGroupKey, bsSummary, sectionDisplayRows, type BsDisplayRow} from "./balanceSheetRows";

const REPORT = decodeBalanceSheetReport(GROUPED_BALANCE_SHEET);
const [ASSETS, , EQUITY] = REPORT.sections;

/** Journal-derived display styles, as `reportStyles` would supply them. */
const STYLES: ReadonlyMap<string, AmountStyle> = new Map<string, AmountStyle>([
    ["$", {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: [",", [3]]}],
    ["GLD", {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}],
    ["TSLA", {side: "R", spaced: true, precision: 1, decimalPoint: ".", digitGroups: null}],
    ["EUR", {side: "R", spaced: true, precision: 2, decimalPoint: ",", digitGroups: [".", [3]]}],
]);

const NONE = (): boolean => false;
const ALL = (): boolean => true;
const only =
    (...keys: string[]) =>
    (key: string): boolean =>
        keys.includes(key);

/** The two row kinds that existed before the current/non-current axis did. */
const isGroupOrAccount = (row: BsDisplayRow): boolean => row.kind === "group" || row.kind === "account";

/** A row minus the two things the axis touched: its `term`, and its key. */
const identity = (row: BsDisplayRow): Partial<BsDisplayRow> => {
    const copy: Partial<BsDisplayRow> = {...row};
    delete copy.term;
    delete copy.key;
    return copy;
};

describe("UNIT reports/ui/balanceSheetRows — sectionDisplayRows", () => {
    it("shows one row per group when everything is collapsed", () => {
        const rows = sectionDisplayRows(ASSETS, NONE);

        expect(rows.map((r) => r.label)).toEqual(["Cash and cash equivalents", "Investments"]);
        expect(rows.every((r) => r.kind === "group")).toBe(true);
        // Collapsed is the DEFAULT, and a collapsed group still has to be
        // cursorable — otherwise `j` on a freshly-loaded report does nothing at
        // all, because no account row exists yet.
        expect(rows).toHaveLength(2);
    });

    it("carries each group's own subtotal, not a row's balance", () => {
        const [cash] = sectionDisplayRows(ASSETS, NONE);
        expect(cash.amount.get("$")).toEqual({m: 4245024n, p: 2});
        expect(cash.account).toBeNull(); // a group heading is not an account
    });

    it("expands one group without expanding its neighbours", () => {
        const rows = sectionDisplayRows(ASSETS, only(bsGroupKey("assets", "Cash and cash equivalents")));

        // Labels are relative to the displayed PARENT, as in every other report
        // table: the group's own root keeps its full path, its children are named
        // by the segments below it. `wise:eur` is one row, not two: the engine
        // is asked for an unclamped report now, and `assets:bank:wise` has no
        // postings of its own, so `compressSectionRows` folds the chain.
        expect(rows.map((r) => r.label)).toEqual(["Cash and cash equivalents", "assets:bank", "checking", "savings", "wise:eur", "Investments"]);
        expect(rows[0].expanded).toBe(true);
        expect(rows[5].expanded).toBe(false);
        expect(rows[5].kind).toBe("group"); // the neighbour is still just its heading
    });

    it("compresses single-child chains exactly as every other report table does", () => {
        // `assets:broker` has one child and nothing of its own, so the pair reads
        // as one row rather than two lines carrying the identical balance.
        const rows = sectionDisplayRows(ASSETS, only(bsGroupKey("assets", "Investments")));
        const accounts = rows.filter((r) => r.kind === "account");

        expect(accounts.map((r) => r.account)).toEqual(["assets:broker:taxable"]);
        expect(accounts[0].label).toBe("assets:broker:taxable");
    });

    it("indents account rows beneath their group", () => {
        const rows = sectionDisplayRows(ASSETS, ALL);

        expect(rows.filter((r) => r.kind === "group").every((r) => r.indent === 0)).toBe(true);
        expect(rows.filter((r) => r.kind === "account").every((r) => r.indent >= 1)).toBe(true);
    });

    it("gives every row a key unique across the whole report", () => {
        const keys = REPORT.sections.flatMap((section) => sectionDisplayRows(section, ALL)).map((r) => r.key);
        expect(new Set(keys).size).toBe(keys.length);
    });

    it("marks a computed group unexpandable so it renders without a dead triangle", () => {
        // "Retained earnings" summarizes accounts that are not on the balance
        // sheet at all: a total and no rows. A disclosure that opens onto
        // nothing is worse than no disclosure.
        const rows = sectionDisplayRows(EQUITY, ALL);
        const retained = rows.find((r) => r.label === "Retained earnings");

        expect(retained?.expandable).toBe(false);
        expect(retained?.expanded).toBe(false);
        // …and asking for it to be open adds no rows at all, so a stray key in
        // the collapse set cannot produce a group that claims to be expanded.
        const asked = sectionDisplayRows(EQUITY, only(bsGroupKey("equity", "Retained earnings")));
        expect(asked).toHaveLength(sectionDisplayRows(EQUITY, NONE).length);
        expect(asked.every((r) => r.expanded === false)).toBe(true);
    });

    it("renders a section with no groups as an empty list, not a throw", () => {
        const empty: BsSection = {kind: "liabilities", title: "Liabilities", groups: [], subsections: [], total: new Map()};
        expect(sectionDisplayRows(empty, ALL)).toEqual([]);
    });
});

// The current/non-current axis. Its whole design claim is that it is ADAPTIVE:
// the rows above are what a journal that classifies nothing must keep getting,
// to the field, and the rows below only exist because a `bsterm:` tag put them
// there.
describe("UNIT reports/ui/balanceSheetRows — current / non-current bands", () => {
    const CLASSIFIED = decodeBalanceSheetReport(CLASSIFIED_BALANCE_SHEET);
    const [BANDED_ASSETS, BANDED_LIABILITIES, BANDED_EQUITY] = CLASSIFIED.sections;

    /** `[kind, label]` per row — the shape of a section, without its figures. */
    const shape = (section: BsSection, isExpanded: (key: string) => boolean = NONE): [string, string][] =>
        sectionDisplayRows(section, isExpanded).map((r) => [r.kind, r.label]);

    describe("a journal that classifies nothing", () => {
        it("emits not one extra row — the adaptive guarantee, stated as a test", () => {
            // The engine sends `subsections: []` and `term: null` throughout for
            // an untagged journal, and the ONLY correct rendering of that is
            // today's: no headings, no band subtotals, no blank "Current" over an
            // undivided list. If this fails, every existing balance sheet moved.
            for (const section of REPORT.sections) {
                const kinds = new Set(sectionDisplayRows(section, ALL).map((r) => r.kind));
                expect([...kinds].sort()).toEqual(["account", "group"]);
            }
            expect(shape(ASSETS)).toEqual([
                ["group", "Cash and cash equivalents"],
                ["group", "Investments"],
            ]);
        });

        it("leaves every row's term null rather than guessing one from the account", () => {
            // Guessing is precisely what [[account-type-not-name]] forbids: a
            // classification that decides which subtotal a balance falls under
            // must come from the declaration, never from the word "cash".
            expect(sectionDisplayRows(ASSETS, ALL).every((r) => r.term === null)).toBe(true);
        });

        it("is what dropping the classification from a banded section returns to", () => {
            // The strongest form of the guarantee: same section, same groups,
            // same accounts — only `subsections` emptied and the terms nulled.
            // Every group and account row comes back identical, field for field,
            // so the bands ADD rows and move nothing.
            const unclassified: BsSection = {
                ...BANDED_ASSETS,
                groups: BANDED_ASSETS.groups.map((group) => ({...group, term: null})),
                subsections: [],
            };
            const plain = sectionDisplayRows(unclassified, ALL);
            const banded = sectionDisplayRows(BANDED_ASSETS, ALL).filter(isGroupOrAccount);

            expect(plain.map(identity)).toEqual(banded.map(identity));
            // The only two things that move are the two that ARE the
            // classification: a row's term, and the term segment in its key.
            expect(plain.every((r) => r.term === null)).toBe(true);
            expect(banded.map((r) => r.key.replace("/current/", "/").replace("/noncurrent/", "/"))).toEqual(plain.map((r) => r.key));
        });
    });

    describe("a journal that classifies its accounts", () => {
        it("opens a band before its first group and closes it after its last", () => {
            expect(shape(BANDED_ASSETS)).toEqual([
                ["subsection", "Current"],
                ["group", "Cash and cash equivalents"],
                ["group", "Accounts receivable"],
                ["subtotal", "Total current assets"],
                ["subsection", "Non-current"],
                ["group", "Property"],
                ["group", "Long-term investments"],
                ["subtotal", "Total non-current assets"],
            ]);
        });

        it("closes a band BELOW the accounts of its last group, not above them", () => {
            // An expanded disclosure belongs to the group it hangs off, so a
            // subtotal printed above those accounts would read as excluding them.
            expect(shape(BANDED_LIABILITIES, ALL)).toEqual([
                ["subsection", "Current"],
                ["group", "Credit cards"],
                // `liabilities:cc` holds nothing itself, so the chain folds.
                ["account", "liabilities:cc:visa"],
                ["group", "Accounts payable"],
                ["account", "liabilities:ap"],
                ["subtotal", "Total current liabilities"],
                ["subsection", "Non-current"],
                ["group", "Long-term debt"],
                ["account", "liabilities:mortgage"],
                ["subtotal", "Total non-current liabilities"],
            ]);
        });

        it("takes the heading and the subtotal label from the engine verbatim", () => {
            // Not composed here from a term and a section title. That mapping
            // would then exist in this module AND in the xlsx builder, which is
            // the duplication the wire field exists to prevent — and the first
            // thing to disagree would be a renamed section.
            const [heading, , , subtotal] = sectionDisplayRows(BANDED_ASSETS, NONE);
            expect(heading.label).toBe(BANDED_ASSETS.subsections[0].heading);
            expect(subtotal.label).toBe(BANDED_ASSETS.subsections[0].label);
        });

        it("passes the engine's subtotal through untouched instead of summing the group lines", () => {
            // Identity, not equality: the row IS the engine's amount. Group
            // totals are rounded for display, and re-adding them is how a band
            // comes to read a cent off the section total printed below it.
            const rows = sectionDisplayRows(BANDED_ASSETS, NONE);
            const [current, noncurrent] = rows.filter((r) => r.kind === "subtotal");

            expect(current.amount).toBe(BANDED_ASSETS.subsections[0].total);
            expect(noncurrent.amount).toBe(BANDED_ASSETS.subsections[1].total);
            expect(current.amount.get("$")).toEqual({m: 6250000n, p: 2}); // 50,000 + 12,500
        });

        it("gives a subheading no figure of its own", () => {
            // The band's total is one row below it. Printing it twice invites the
            // reader to look for a difference between the two.
            const [heading] = sectionDisplayRows(BANDED_ASSETS, NONE);
            expect(heading.amount.size).toBe(0);
            expect(heading.account).toBeNull();
        });

        it("compresses a single-child chain inside a band exactly as everywhere else", () => {
            const rows = sectionDisplayRows(BANDED_ASSETS, only(bsGroupKey("assets", "Property", "noncurrent")));
            const accounts = rows.filter((r) => r.kind === "account");

            expect(accounts.map((r) => r.account)).toEqual(["assets:property:house"]);
        });

        it("marks each group with the band it fell in", () => {
            const groups = sectionDisplayRows(BANDED_ASSETS, NONE).filter((r) => r.kind === "group");
            expect(groups.map((r) => [r.label, r.term])).toEqual([
                ["Cash and cash equivalents", "current"],
                ["Accounts receivable", "current"],
                ["Property", "noncurrent"],
                ["Long-term investments", "noncurrent"],
            ]);
        });

        it("never bands equity, even in a report whose other boxes are banded", () => {
            expect(shape(BANDED_EQUITY)).toEqual([
                ["group", "Opening"],
                ["group", "Retained earnings"],
            ]);
        });

        it("keeps every key unique across the whole report", () => {
            const keys = CLASSIFIED.sections.flatMap((section) => sectionDisplayRows(section, ALL)).map((r) => r.key);
            expect(new Set(keys).size).toBe(keys.length);
        });

        describe("one group name on both sides of the axis", () => {
            // The engine keys groups by (term, name), so a `bsgroup:` whose
            // accounts are partly current and partly not IS two lines — a
            // receivable due this year and one due in five belong under
            // different subheadings.
            const [STRADDLED] = decodeBalanceSheetReport(STRADDLING_BALANCE_SHEET).sections;

            it("prints it once under each band", () => {
                expect(shape(STRADDLED)).toEqual([
                    ["subsection", "Current"],
                    ["group", "Cash and cash equivalents"],
                    ["group", "Accounts receivable"],
                    ["subtotal", "Total current assets"],
                    ["subsection", "Non-current"],
                    ["group", "Property"],
                    ["group", "Accounts receivable"],
                    ["subtotal", "Total non-current assets"],
                ]);
            });

            it("gives the two lines different keys, so they are two lines in every sense", () => {
                // Section + name alone collided here. One key does three jobs —
                // `{#each}` identity, the collapse set, the cursor anchor — so a
                // collision is a duplicate key, a shared disclosure and a shared
                // cursor stop all at once.
                const receivables = sectionDisplayRows(STRADDLED, NONE).filter((r) => r.label === "Accounts receivable");

                expect(receivables).toHaveLength(2);
                expect(receivables[0].key).not.toBe(receivables[1].key);
                expect(receivables.map((r) => r.term)).toEqual(["current", "noncurrent"]);
            });

            it("opens only the one that was asked for", () => {
                const rows = sectionDisplayRows(STRADDLED, only(bsGroupKey("assets", "Accounts receivable", "noncurrent")));
                const receivables = rows.filter((r) => r.label === "Accounts receivable");

                expect(receivables.map((r) => r.expanded)).toEqual([false, true]);
                // …and the accounts revealed are that line's, not the other's.
                expect(rows.filter((r) => r.kind === "account").map((r) => r.account)).toEqual(["assets:broker:ira"]);
            });
        });
    });

    describe("a body that breaks the engine's own invariants", () => {
        // None of these can happen per the contract. They are here because the
        // failure mode that matters is a LOST ROW: whatever the section says
        // about bands, every group must still print its line.
        const band = (term: string, heading: string, label: string): BsSection["subsections"][number] =>
            ({term, heading, label, total: new Map()}) as BsSection["subsections"][number];

        it("still prints a group whose term names no band", () => {
            const orphan: BsSection = {
                ...BANDED_ASSETS,
                subsections: [band("current", "Current", "Total current assets")],
            };
            expect(shape(orphan)).toEqual([
                ["subsection", "Current"],
                ["group", "Cash and cash equivalents"],
                ["group", "Accounts receivable"],
                ["subtotal", "Total current assets"],
                // No heading and no subtotal for a band that was never declared —
                // but both groups are on screen, which is the part that matters.
                ["group", "Property"],
                ["group", "Long-term investments"],
            ]);
        });

        it("still prints a group with no term at all", () => {
            const [cash, ...rest] = BANDED_ASSETS.groups;
            const mixed: BsSection = {...BANDED_ASSETS, groups: [{...cash, term: null}, ...rest]};

            expect(shape(mixed)).toEqual([
                ["group", "Cash and cash equivalents"],
                ["subsection", "Current"],
                ["group", "Accounts receivable"],
                ["subtotal", "Total current assets"],
                ["subsection", "Non-current"],
                ["group", "Property"],
                ["group", "Long-term investments"],
                ["subtotal", "Total non-current assets"],
            ]);
        });

        it("emits no heading for a band with no groups", () => {
            // A subheading standing over nothing is the one output that would be
            // read as data loss rather than as an empty section.
            const phantom: BsSection = {...BANDED_EQUITY, subsections: [band("noncurrent", "Non-current", "Total non-current equity")]};
            expect(shape(phantom)).toEqual([
                ["group", "Opening"],
                ["group", "Retained earnings"],
            ]);
        });

        describe("groups of one term arriving non-contiguously", () => {
            // Terms [current, noncurrent, current]: the ordering invariant the
            // one-pass walk leans on, broken. The degradation is to RE-OPEN the
            // band — same heading, same engine subtotal, in the engine's own
            // order — never to move the stray group up to "repair" the run:
            // this module reorders nothing anywhere else, and a band that
            // visibly opens twice is honest about the input where a quietly
            // relocated group is not.
            const [cash, receivable, property] = BANDED_ASSETS.groups;
            const NONCONTIGUOUS: BsSection = {...BANDED_ASSETS, groups: [cash, property, receivable]};

            it("re-opens the band around the stray group instead of losing or moving it", () => {
                expect(shape(NONCONTIGUOUS)).toEqual([
                    ["subsection", "Current"],
                    ["group", "Cash and cash equivalents"],
                    ["subtotal", "Total current assets"],
                    ["subsection", "Non-current"],
                    ["group", "Property"],
                    ["subtotal", "Total non-current assets"],
                    ["subsection", "Current"],
                    ["group", "Accounts receivable"],
                    ["subtotal", "Total current assets"],
                ]);
            });

            it("keys the re-opened band by occurrence, so no two rows share a key", () => {
                // Without the occurrence counter both Current bands minted
                // "assets/@current" (and its "/total"), and a duplicate key is
                // not the cosmetic flaw an old comment here called it: Svelte 5's
                // keyed {#each} throws `each_key_duplicate` in dev and misrenders
                // in prod — the whole statement blanked, a strictly worse outcome
                // than the contract break that provoked it.
                const rows = sectionDisplayRows(NONCONTIGUOUS, NONE);
                const keys = rows.map((r) => r.key);

                expect(new Set(keys).size).toBe(keys.length);
                expect(rows.filter((r) => r.kind === "subsection").map((r) => r.key)).toEqual(["assets/@current", "assets/@noncurrent", "assets/@current#2"]);
                expect(rows.filter((r) => r.kind === "subtotal").map((r) => r.key)).toEqual([
                    "assets/@current/total",
                    "assets/@noncurrent/total",
                    "assets/@current#2/total",
                ]);
            });

            it("keeps a well-formed report's band keys exactly as they were", () => {
                // Occurrence 1 is unsuffixed on purpose: the counter exists for a
                // report that cannot happen, and must cost a contract-keeping one
                // nothing — not even its key strings.
                const rows = sectionDisplayRows(BANDED_ASSETS, NONE);
                expect(rows.filter((r) => r.kind !== "group").map((r) => r.key)).toEqual([
                    "assets/@current",
                    "assets/@current/total",
                    "assets/@noncurrent",
                    "assets/@noncurrent/total",
                ]);
            });
        });
    });

    describe("bsCursorRows", () => {
        it("leaves the band rows out, so `j` never stops where Enter does nothing", () => {
            // The same rule the income statement applies to its ladder lines: a
            // heading and a subtotal can be neither expanded nor drilled into.
            const rows = sectionDisplayRows(BANDED_ASSETS, ALL);
            expect(rows.some((r) => r.kind === "subsection")).toBe(true);
            expect(bsCursorRows(rows).map((r) => r.kind)).toEqual(rows.filter(isGroupOrAccount).map((r) => r.kind));
        });

        it("is a filtering of the very array the template iterates, in the same order", () => {
            const rows = sectionDisplayRows(BANDED_ASSETS, ALL);
            const cursorable = bsCursorRows(rows);

            expect(cursorable).toEqual(rows.filter(isGroupOrAccount));
            expect(cursorable.every((row) => rows.includes(row))).toBe(true);
        });
    });
});

describe("UNIT reports/ui/balanceSheetRows — amountCell", () => {
    const usd = (m: number, p: number): MixedAmount => new Map([["$", dec(m, p)]]);

    it("promotes the base commodity to the one figure on the line", () => {
        expect(amountCell(usd(4245024, 2), "$", STYLES)).toEqual({text: "$42,450.24", negative: false, extras: []});
    });

    it("rounds half away from zero, matching every other money surface", () => {
        // $59,612.615 — the engine's exact assets total. hledger's CLI prints
        // .61 here because Haskell's `round` is half-to-EVEN; `formatDec` is
        // half-away-from-zero everywhere in this app, so .62 is the number the
        // screen and the workbook both show.
        expect(amountCell(usd(59612615, 3), "$", STYLES).text).toBe("$59,612.62");
    });

    it("demotes what the valuation could not convert to a secondary line", () => {
        const cell = amountCell(REPORT.sections[0].total, "$", STYLES);

        expect(cell.text).toBe("$59,612.62");
        // Sorted, so the footnote never depends on Map insertion order.
        expect(cell.extras).toEqual(["5.0 GLD", "-2.0 TSLA"]);
    });

    it("flags a negative base figure for the caller to paint", () => {
        expect(amountCell(usd(-53115, 2), "$", STYLES)).toMatchObject({text: "$-531.15", negative: true});
    });

    it("shows a real formatted zero when the amount has no base part", () => {
        // "Transfers" is 5 GLD and no dollars. A blank cell would read as "no
        // data"; `$0.00` with the GLD footnote reads as what it is.
        const transfers = REPORT.sections[2].groups.find((g) => g.name === "Transfers");
        expect(amountCell(transfers?.total ?? new Map(), "$", STYLES)).toEqual({text: "$0.00", negative: false, extras: ["5.0 GLD"]});
    });

    it("drops zero commodities from the secondary line", () => {
        const withZero: MixedAmount = new Map([
            ["$", dec(100, 2)],
            ["GLD", dec(0, 0)],
        ]);
        expect(amountCell(withZero, "$", STYLES).extras).toEqual([]);
    });

    describe("a journal with no base commodity", () => {
        // `base` is `Option<Commodity>` on the wire and arrives null. There is
        // then nothing to promote, so the first commodity leads and the rest
        // stay footnotes — deterministic, and honest that nothing was converted.
        it("leads with the first commodity in sort order", () => {
            const mixed: MixedAmount = new Map([
                ["GLD", dec(5, 0)],
                ["EUR", dec(56675, 2)],
            ]);
            expect(amountCell(mixed, null, STYLES)).toEqual({text: "566,75 EUR", negative: false, extras: ["5.0 GLD"]});
        });

        it("renders a bare 0 for an empty amount rather than a currency it does not have", () => {
            expect(amountCell(new Map(), null, STYLES)).toEqual({text: "0", negative: false, extras: []});
        });
    });
});

describe("UNIT reports/ui/balanceSheetRows — bsSummary", () => {
    it("adds liabilities and equity on the exact Decs, per commodity", () => {
        const summary = bsSummary(REPORT);

        // $531.15 (places 2) + $59,081.465 (places 3) = $59,612.615, carried at
        // full precision — the half-cent survives the addition. Adding the
        // DISPLAYED $531.15 and $59,081.47 instead gives $59,612.62, which is
        // right here by luck and wrong the moment either side rounds the other
        // way.
        expect(summary.liabilitiesPlusEquity.get("$")).toEqual({m: 59612615n, p: 3});
        // The unpriced holdings tie out too, and must not vanish from the line.
        expect(summary.liabilitiesPlusEquity.get("GLD")).toEqual({m: 5n, p: 0});
        expect(summary.liabilitiesPlusEquity.get("TSLA")).toEqual({m: -2n, p: 0});
        expect(summary.liabilitiesPlusEquity).toEqual(summary.assets);
    });

    it("takes the verdict from the engine, not from the tie-out it displays", () => {
        expect(bsSummary(REPORT).balanced).toBe(true);

        // Same sections, so `liabilitiesPlusEquity` still equals `assets` to the
        // last decimal — the imbalance is only in `check`. Deriving the verdict
        // by comparing the two displayed figures would report this as balanced.
        const unbalanced = bsSummary(decodeBalanceSheetReport(UNBALANCED_BALANCE_SHEET));
        expect(unbalanced.liabilitiesPlusEquity).toEqual(unbalanced.assets);
        expect(unbalanced.balanced).toBe(false);
    });

    it("does not re-derive the verdict from `check`, which is dust on a real journal", () => {
        // A journal holding fractional lots leaves sub-cent residue in `check`
        // with nothing wrong: `26.2690 VTI @ $289.7713` costs $7,612.00227970
        // and no cash posting can carry the surplus digits. hledger accepts such
        // a journal; so does the engine, which is why it sends `balanced: true`
        // beside a non-empty `check`. `maIsZero(check)` here is what made a
        // valid journal warn "should be zero, but it is $0.00227970".
        const dusty = decodeBalanceSheetReport({...(GROUPED_BALANCE_SHEET as object), check: {$: {mantissa: "22797", places: 7}}, balanced: true});
        expect(dusty.check.size).toBe(1);
        expect(bsSummary(dusty).balanced).toBe(true);
    });

    it("finds each section by kind, so a reordered report cannot mislabel a figure", () => {
        const reversed = {...REPORT, sections: [...REPORT.sections].reverse()};

        expect(bsSummary(reversed)).toEqual(bsSummary(REPORT));
    });

    it("treats a missing section as zero rather than reading the wrong one", () => {
        const noEquity = {...REPORT, sections: REPORT.sections.filter((s) => s.kind !== "equity")};
        const summary = bsSummary(noEquity);

        expect(summary.equity).toEqual(new Map());
        expect(summary.liabilitiesPlusEquity).toEqual(summary.liabilities);
    });
});
