// The review card's pure decisions (plan 32 D6, D7, D13, D20, D21).

import {describe, expect, it} from "vitest";
import {reviewFormOf, type CardChange} from "#lib/api/editMapping.js";
import type {AccountClass, AccountClasses} from "#lib/domain/accountClass.js";
import {dec} from "#lib/domain/money.js";
import type {Amount, Posting, Transaction} from "#lib/domain/types.js";
import {
    acceptAddChip,
    acceptChip,
    addChips,
    addDirty,
    balancingHint,
    blankItem,
    blankReviewForm,
    changeSummary,
    checkState,
    chipsFor,
    cycleChip,
    detailsDirty,
    detailsSummary,
    footerState,
    hasUnknown,
    heroOf,
    isDirty,
    isUncategorized,
    longDate,
    minusDays,
    NEW_TXN,
    recurringLabel,
    reviewBadge,
    reviewedBox,
    rowMarker,
    rowShadow,
    similarRows,
    splitDescription,
    toggleReviewed,
} from "./card";
import type {QueueItem, ReviewTxn, Suggestion} from "./types";

const style = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null} as const;
const usd = (cents: number): Amount => ({commodity: "$", qty: dec(cents, 2), style});
const posting = (account: string, cents: number | null, extra: Partial<Posting> = {}): Posting => ({
    account,
    amounts: cents === null ? [] : [usd(cents)],
    status: "unmarked",
    comment: "",
    tags: [],
    ...extra,
});
const txn = (index: number, description: string, postings: Posting[], extra: Partial<Transaction> = {}): Transaction => ({
    index,
    date: "2026-10-12",
    status: "unmarked",
    description,
    code: "",
    comment: "",
    tags: [],
    postings,
    ...extra,
});
const cls = (type: AccountClass["type"]): AccountClass => ({type, typeDeclared: true, term: null, termGuessed: false});
const CLASSES: AccountClasses = new Map([
    ["assets:checking", cls("cash")],
    ["liabilities:visa", cls("liability")],
    ["expenses", cls("expense")],
    ["expenses:unknown", cls("expense")],
    ["expenses:food:coffee", cls("expense")],
    ["income", cls("revenue")],
    ["income:salary", cls("revenue")],
]);
const suggestion = (account: string, strong: boolean, extra: Partial<Suggestion> = {}): Suggestion => ({
    account,
    description: null,
    source: "exactKey",
    confidence: strong ? 8000 : 3000,
    strong,
    count: 4,
    ...extra,
});
const item = (t: Transaction, suggestions: Suggestion[] = []): QueueItem => ({
    txn: t,
    flags: [],
    unknown: true,
    reviewed: false,
    fingerprint: "fp",
    key: "k",
    suggestions,
});
const reviewTxn = (t: Transaction): ReviewTxn => ({txn: t, flags: [], unknown: true, reviewed: false, fingerprint: "fp"});

const coffee = txn(1, "SQ *BLUE BOTTLE 4412 OAKLAND", [posting("expenses:unknown", 675), posting("liabilities:visa", -675)]);

describe("UNIT isUncategorized", () => {
    it("matches checks.rs: an unknown/uncategorized leaf, or a bare expense or revenue account", () => {
        expect(isUncategorized("expenses:unknown", CLASSES)).toBe(true);
        expect(isUncategorized("Expenses:Uncategorized", CLASSES)).toBe(true);
        expect(isUncategorized("expenses", CLASSES)).toBe(true);
        expect(isUncategorized("income", CLASSES)).toBe(true);
        expect(isUncategorized("expenses:food:coffee", CLASSES)).toBe(false);
        expect(isUncategorized("assets", CLASSES)).toBe(false);
        expect(isUncategorized("", CLASSES)).toBe(false);
    });

    it("counts a generated unknown posting, which only its rule can fix (D3)", () => {
        const generated = txn(2, "Coffee", [
            posting("expenses:food:coffee", 500),
            posting("liabilities:visa", -500),
            posting("expenses:unknown", 500, {generated: {rule: 0, query: "desc:coffee$"}}),
        ]);
        expect(hasUnknown(generated, reviewFormOf(generated), CLASSES)).toBe(true);
        const form = reviewFormOf(coffee);
        expect(hasUnknown(coffee, form, CLASSES)).toBe(true);
        form.postings[0]!.account = "expenses:food:coffee";
        expect(hasUnknown(coffee, form, CLASSES)).toBe(false);
    });
});

describe("UNIT heroOf", () => {
    it("splits payee and note, and colours money out, money in and transfers", () => {
        expect(splitDescription("Safeway | weekly shop")).toEqual({payee: "Safeway", note: "weekly shop"});
        expect(heroOf(coffee, CLASSES)).toMatchObject({
            payee: "SQ *BLUE BOTTLE 4412 OAKLAND",
            tone: "out",
            funding: {preposition: "from", account: "liabilities:visa"},
        });
        const pay = txn(2, "Payroll", [posting("assets:checking", 100000), posting("income:salary", -100000)]);
        expect(heroOf(pay, CLASSES)).toMatchObject({tone: "in", funding: {preposition: "to", account: "assets:checking"}});
        const move = txn(3, "Pay the card", [posting("liabilities:visa", 5000), posting("assets:checking", -5000)]);
        expect(heroOf(move, CLASSES)).toMatchObject({tone: "transfer", funding: {preposition: "from", account: "assets:checking"}});
        expect(heroOf(coffee, CLASSES).amounts.map((a) => a.qty)).toEqual([dec(675, 2)]);
    });

    it("signs by the net on the user's own accounts: a stock sale with a fee is money in, not out", () => {
        const shares: Amount = {commodity: "AAPL", qty: dec(-10, 0), style, cost: {commodity: "$", qty: dec(15000, 2), per: true}};
        const sale = txn(4, "Sell AAPL", [
            posting("assets:brokerage:aapl", null, {amounts: [shares]}),
            posting("assets:checking", 149500),
            posting("expenses:fees", 500),
            posting("income:gains", -1000),
        ]);
        const classes: AccountClasses = new Map([
            ...CLASSES,
            ["assets:brokerage:aapl", cls("asset")],
            ["expenses:fees", cls("expense")],
            ["income:gains", cls("revenue")],
        ]);
        const hero = heroOf(sale, classes);
        expect(hero).toMatchObject({tone: "in", funding: {preposition: "to", account: "assets:checking"}});
        expect(hero.amounts.map((a) => [a.commodity, a.qty])).toEqual([["$", dec(149500, 2)]]);
    });

    it("shows what landed, net of withholding, for a paycheck", () => {
        const classes: AccountClasses = new Map([...CLASSES, ["expenses:tax", cls("expense")]]);
        const pay = txn(5, "Payroll", [posting("income:salary", -500000), posting("expenses:tax", 100000), posting("assets:checking", 400000)]);
        const hero = heroOf(pay, classes);
        expect(hero.tone).toBe("in");
        expect(hero.amounts.map((a) => a.qty)).toEqual([dec(400000, 2)]);
    });

    it("a refund to the card is money in", () => {
        const refund = txn(6, "Refund", [posting("expenses:food:coffee", -675), posting("liabilities:visa", 675)]);
        expect(heroOf(refund, CLASSES)).toMatchObject({tone: "in", funding: {preposition: "to", account: "liabilities:visa"}});
    });

    it("formats dates as calendar dates whatever the zone", () => {
        expect(longDate("2026-10-12")).toBe("Oct 12, 2026");
        expect(minusDays("2026-10-30", 365)).toBe("2025-10-30");
        expect(minusDays("2026-03-01", 1)).toBe("2026-02-28");
    });
});

describe("UNIT footer and dirty", () => {
    it("names D7's four states", () => {
        expect(footerState(false, false)).toBe("clean");
        expect(footerState(false, true)).toBe("clean-unknown");
        expect(footerState(true, false)).toBe("dirty");
        expect(footerState(true, true)).toBe("dirty-unknown");
    });

    it("is dirty by value: typing a change and undoing it is clean again", () => {
        const form = reviewFormOf(coffee);
        expect(isDirty(coffee, form)).toBe(false);
        form.description = "Blue Bottle";
        expect(isDirty(coffee, form)).toBe(true);
        form.description = coffee.description;
        expect(isDirty(coffee, form)).toBe(false);
    });
});

describe("UNIT details row (D33)", () => {
    it("summarises the form's header, leaving out what is unset", () => {
        const form = reviewFormOf(coffee);
        expect(detailsSummary(form)).toEqual(["Oct 12, 2026"]);
        expect(detailsSummary({...form, date2: "2026-10-14", status: "cleared", code: " 1042 "})).toEqual([
            "Oct 12, 2026",
            "secondary Oct 14, 2026",
            "cleared",
            "#1042",
        ]);
        expect(detailsSummary({...form, date: ""})).toEqual(["no date"]);
    });

    it("is dirty when the date, secondary date, status or code moves, and only then", () => {
        const original = reviewFormOf(coffee);
        expect(detailsDirty(original, {...original, description: "Blue Bottle"})).toBe(false);
        expect(detailsDirty(original, {...original, date: "2026-10-13"})).toBe(true);
        expect(detailsDirty(original, {...original, date2: "2026-10-14"})).toBe(true);
        expect(detailsDirty(original, {...original, status: "pending"})).toBe(true);
        expect(detailsDirty(original, {...original, code: "7"})).toBe(true);
        expect(detailsDirty(original, {...original, code: "  "})).toBe(false);
    });
});

describe("UNIT balancingHint (D35)", () => {
    const elided = txn(9, "Blue Bottle", [posting("expenses:food:coffee", 675), posting("liabilities:visa", -675, {elided: true})]);
    const original = (): ReturnType<typeof reviewFormOf>["postings"] => reviewFormOf(elided).postings;

    it("shows what the blank leg balances to, and follows an amount typed elsewhere", () => {
        const rows = original();
        expect(balancingHint(original(), rows, 1)).toBe("auto · -6.75");
        (rows[0] as (typeof rows)[number]).amount = "7.5";
        expect(balancingHint(original(), rows, 1)).toBe("auto · -7.50");
    });

    it("is plain auto on a row with an amount, and while another row is blank too", () => {
        const rows = original();
        expect(balancingHint(original(), rows, 0)).toBe("auto");
        (rows[0] as (typeof rows)[number]).amount = "";
        expect(balancingHint(original(), rows, 1)).toBe("auto");
    });

    it("falls back to the engine's value while nothing changed, when the form can't work it out", () => {
        const priced = txn(10, "Buy", [
            posting("assets:broker", null, {amounts: [{commodity: "VTI", qty: dec(2, 0), style, cost: {commodity: "$", qty: dec(10000, 2), per: true}}]}),
            posting("assets:cash", -20000, {elided: true}),
        ]);
        const rows = reviewFormOf(priced).postings;
        expect(balancingHint(reviewFormOf(priced).postings, rows, 1)).toBe("auto · -200.00");
        (rows[0] as (typeof rows)[number]).amount = "3";
        expect(balancingHint(reviewFormOf(priced).postings, rows, 1)).toBe("auto");
    });

    it("works nothing out across commodities", () => {
        const rows = original();
        (rows[1] as (typeof rows)[number]).commodity = "EUR";
        (rows[0] as (typeof rows)[number]).amount = "7.5";
        expect(balancingHint(original(), rows, 1)).toBe("auto");
    });
});

describe("UNIT suggestion chips", () => {
    it("offers an unknown card's suggestions on its unknown row, pre-selecting only a strong top one", () => {
        const strong = chipsFor(
            item(coffee, [suggestion("expenses:food:coffee", true), suggestion("expenses:food:groceries", false)]),
            reviewFormOf(coffee),
            CLASSES
        );
        expect(strong).toMatchObject({target: 0, initial: 0, mode: "fix"});
        expect(strong.chips.map((chip) => chip.account)).toEqual(["expenses:food:coffee", "expenses:food:groceries"]);
        const weak = chipsFor(item(coffee, [suggestion("expenses:food:coffee", false)]), reviewFormOf(coffee), CLASSES);
        expect(weak.initial).toBeNull();
        expect(chipsFor(item(coffee), reviewFormOf(coffee), CLASSES).chips).toEqual([]);
    });

    it("offers a categorised card only a strong disagreement, never pre-selected", () => {
        const groceries = txn(4, "SQ *BLUE BOTTLE 3300 OAKLAND", [posting("expenses:food:groceries", 1200), posting("liabilities:visa", -1200)]);
        const disagree = chipsFor(item(groceries, [suggestion("expenses:food:coffee", true)]), reviewFormOf(groceries), CLASSES);
        expect(disagree).toMatchObject({target: 0, initial: null, mode: "usually"});
        const agrees = chipsFor(item(groceries, [suggestion("expenses:food:groceries", true)]), reviewFormOf(groceries), CLASSES);
        expect(agrees.chips).toEqual([]);
    });

    it("cycles top → alt 1 → alt 2 → top, filling the first time", () => {
        const chips = [suggestion("a:one", true), suggestion("a:two", false), suggestion("a:three", false)];
        expect(cycleChip(chips, null, "expenses:unknown")).toBe(0);
        expect(cycleChip(chips, 0, "expenses:unknown")).toBe(0);
        expect(cycleChip(chips, 0, "a:one")).toBe(1);
        expect(cycleChip(chips, 2, "a:three")).toBe(0);
        expect(cycleChip([], null, "")).toBeNull();
    });

    it("accepts a chip's account, and its rename unless the card was renamed already", () => {
        const chip = suggestion("expenses:food:coffee", true, {description: "Blue Bottle Coffee"});
        const accepted = acceptChip(coffee, reviewFormOf(coffee), 0, chip);
        expect(accepted.postings[0]?.account).toBe("expenses:food:coffee");
        expect(accepted.description).toBe("Blue Bottle Coffee");
        const renamed = {...reviewFormOf(coffee), description: "My Coffee"};
        expect(acceptChip(coffee, renamed, 0, chip).description).toBe("My Coffee");
    });
});

describe("UNIT similar rows", () => {
    const sibling = (index: number, account: string): ReviewTxn =>
        reviewTxn(txn(index, `SQ *BLUE BOTTLE ${index} OAKLAND`, [posting(account, 500), posting("liabilities:visa", -500)]));
    const candidates = [sibling(2, "expenses:unknown"), sibling(3, "expenses:food:groceries")];
    const move: CardChange = {description: null, accounts: new Map([["expenses:unknown", "expenses:food:coffee"]])};

    it("lists, for an account change, only the candidates that have the account, with their chips", () => {
        const rows = similarRows(candidates, [2], move);
        expect(rows.map((row) => row.txn.txn.index)).toEqual([2]);
        expect(rows[0]).toMatchObject({rename: null, moves: [["expenses:unknown", "expenses:food:coffee"]], inQueue: true});
        expect(rows[0]?.ops).toEqual([{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}]);
    });

    it("lists every candidate for a rename, each receiving exactly its share", () => {
        const both: CardChange = {description: "Blue Bottle Coffee", accounts: move.accounts};
        const rows = similarRows(candidates, [], both);
        expect(rows.map((row) => [row.rename, row.moves.length])).toEqual([
            ["Blue Bottle Coffee", 1],
            ["Blue Bottle Coffee", 0],
        ]);
    });

    it("lists everything, receiving nothing, before an applicable edit; and reads the header's tri-state", () => {
        const rows = similarRows(candidates, [], null);
        expect(rows.every((row) => row.ops.length === 0)).toBe(true);
        expect(checkState(rows, new Set())).toBe("none");
        expect(checkState(rows, new Set([2]))).toBe("some");
        expect(checkState(rows, new Set([2, 3]))).toBe("all");
        expect(changeSummary(null)).toBe("");
        expect(changeSummary({description: "x", accounts: move.accounts})).toBe("description + account");
    });
});

describe("UNIT review badge (D21)", () => {
    it("reads the header's reviewed tag", () => {
        expect(reviewBadge([["reviewed", "2026-10-30"]])).toEqual({kind: "reviewed", on: "2026-10-30", label: "Reviewed Oct 30"});
        expect(reviewBadge([["reviewed", "yes"]])).toMatchObject({kind: "reviewed", label: "Reviewed"});
        expect(reviewBadge([["reviewed", "ignore"]])).toMatchObject({kind: "ignored", label: "Ignored"});
        expect(reviewBadge([["receipt", "1"]])).toMatchObject({kind: "unreviewed", label: "Unreviewed"});
    });
});

describe("UNIT row stripes (D20)", () => {
    const row = (date: string, unknown: boolean, reviewed: boolean) => ({txn: {date}, unknown, reviewed});
    const TODAY = "2026-10-30";

    it("marks unknown amber and unreviewed faint, reviewed rows not at all", () => {
        expect(rowMarker(row("2026-10-12", true, false), TODAY, 365, true)).toBe("unknown");
        expect(rowMarker(row("2026-10-20", true, true), TODAY, 365, true)).toBe("unknown");
        expect(rowMarker(row("2026-10-25", false, false), TODAY, 365, true)).toBe("unreviewed");
        expect(rowMarker(row("2026-10-18", false, true), TODAY, 365, true)).toBeNull();
    });

    it("stops at the window, inclusive at its far edge, and never marks the future", () => {
        expect(rowMarker(row("2025-10-30", false, false), TODAY, 365, true)).toBe("unreviewed");
        expect(rowMarker(row("2025-10-29", false, false), TODAY, 365, true)).toBeNull();
        expect(rowMarker(row("2025-10-29", true, false), TODAY, 365, true)).toBeNull();
        expect(rowMarker(row("2026-11-15", true, false), TODAY, 365, true)).toBeNull();
        expect(rowMarker(row("2001-01-01", false, false), TODAY, 0, true)).toBe("unreviewed");
    });

    it("is off with the setting", () => {
        expect(rowMarker(row("2026-10-12", true, false), TODAY, 365, false)).toBeNull();
    });

    it("stacks the cursor's rule inside the stripe", () => {
        expect(rowShadow(null, false)).toBe("none");
        expect(rowShadow(null, true)).toBe("inset 3px 0 0 0 var(--color-primary)");
        expect(rowShadow("unknown", false)).toBe("inset 3px 0 0 0 color-mix(in oklab, var(--color-warning) 80%, transparent)");
        expect(rowShadow("unreviewed", true)).toBe(
            "inset 3px 0 0 0 color-mix(in oklab, var(--color-base-content) 15%, transparent), inset 6px 0 0 0 var(--color-primary)"
        );
    });
});

describe("UNIT review card — Mark reviewed (D32)", () => {
    const withTags = (tags: [string, string][]) => ({...blankReviewForm("2026-11-01", "$"), tags});

    it("reads the form's reviewed tag: unchecked, checked, or checked as Ignored", () => {
        expect(reviewedBox(withTags([]))).toEqual({checked: false, label: "Mark reviewed"});
        expect(reviewedBox(withTags([["reviewed", "2026-10-30"]]))).toEqual({checked: true, label: "Mark reviewed"});
        expect(reviewedBox(withTags([["reviewed", "ignore"]]))).toEqual({checked: true, label: "Ignored"});
    });

    it("checking stamps today; unchecking removes the tag", () => {
        const none = withTags([["trip", "nyc"]]);
        expect(toggleReviewed(none, none, "2026-11-01")).toEqual([
            ["trip", "nyc"],
            ["reviewed", "2026-11-01"],
        ]);
        const done = withTags([["reviewed", "2026-10-30"]]);
        expect(toggleReviewed(done, done, "2026-11-01")).toEqual([]);
    });

    it("checking again puts back the tag the transaction had, date or ignore", () => {
        for (const value of ["2026-10-30", "ignore"]) {
            const original = withTags([["reviewed", value]]);
            expect(toggleReviewed(withTags([]), original, "2026-11-01")).toEqual([["reviewed", value]]);
        }
    });
});

describe("UNIT review card — add mode (D36)", () => {
    const fill = {amount: {commodity: "$", qty: dec(1549, 2)}, account: "liabilities:visa"};
    const recurring = (extra: Partial<Suggestion> = {}): Suggestion => ({
        account: "expenses:streaming",
        description: null,
        source: "recurring",
        confidence: 6000,
        strong: true,
        count: 4,
        recurring: fill,
        ...extra,
    });
    const classes: AccountClasses = new Map();

    it("starts blank: today, unmarked, two rows, in no file", () => {
        const item = blankItem("2026-11-01");
        expect(item.txn.index).toBe(NEW_TXN);
        expect(item.fingerprint).toBeNull();
        const form = blankReviewForm("2026-11-01", "€");
        expect(form).toMatchObject({date: "2026-11-01", status: "unmarked", description: "", tags: []});
        expect(form.postings.map((row) => [row.account, row.amount, row.commodity])).toEqual([
            ["", "", "€"],
            ["", "", "€"],
        ]);
        expect(addDirty(form, structuredClone(form))).toBe(false);
        expect(addDirty(form, {...form, description: "x"})).toBe(true);
    });

    it("offers the suggestions on the first row, the strong one selected", () => {
        const item = {
            ...blankItem("2026-11-01"),
            suggestions: [recurring(), recurring({account: "expenses:unknown"}), recurring({account: "expenses:tv", strong: false})],
        };
        const chips = addChips(item, classes);
        expect(chips.chips.map((chip) => chip.account)).toEqual(["expenses:streaming", "expenses:tv"]);
        expect(chips).toMatchObject({target: 0, initial: 0, mode: "fix"});
        expect(addChips(blankItem("2026-11-01"), classes).chips).toEqual([]);
    });

    it("accepting a recurring chip fills the amount and the funding account, and only blank fields", () => {
        const blank = blankReviewForm("2026-11-01", "$");
        const filled = acceptAddChip(blank, 0, recurring());
        expect(filled.postings.map((row) => [row.account, row.amount, row.commodity])).toEqual([
            ["expenses:streaming", "15.49", "$"],
            ["liabilities:visa", "", "$"],
        ]);
        const typed = {...blank, postings: blank.postings.map((row, at) => (at === 0 ? {...row, amount: "17.99"} : {...row, account: "assets:checking"}))};
        expect(acceptAddChip(typed, 0, recurring()).postings.map((row) => [row.account, row.amount])).toEqual([
            ["expenses:streaming", "17.99"],
            ["assets:checking", ""],
        ]);
    });

    it("a chip with no recurring charge sets the account alone, and a rename only on a blank description", () => {
        const blank = blankReviewForm("2026-11-01", "$");
        const chip = recurring({recurring: undefined, description: "Netflix"});
        const filled = acceptAddChip(blank, 0, chip);
        expect(filled.postings.map((row) => [row.account, row.amount])).toEqual([
            ["expenses:streaming", ""],
            ["", ""],
        ]);
        expect(filled.description).toBe("Netflix");
        expect(acceptAddChip({...blank, description: "NFLX"}, 0, chip).description).toBe("NFLX");
    });

    it("labels what a recurring chip fills in", () => {
        expect(recurringLabel(recurring())).toBe("$15.49 from liabilities:visa");
        expect(recurringLabel(recurring({recurring: {...fill, amount: {commodity: "EUR", qty: dec(9, 0)}}}))).toBe("9 EUR from liabilities:visa");
        expect(recurringLabel(recurring({recurring: undefined}))).toBeNull();
    });
});
