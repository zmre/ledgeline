// The review card's save (plan 32 D9, D11): which ops a form becomes. The
// engine applies them (crates/ledgeline-core/tests/edit_batch.rs); what is
// pinned here is the CHOICE — surgical where it can be, `replace` only when a
// split or a date forces it — because a needless `replace` reformats the
// user's text and loses an elided amount.

import {describe, expect, it} from "vitest";
import type {AmountStyle, Posting, Transaction} from "#lib/domain/types.js";
import type {ReviewForm} from "#lib/review/types.js";
import {addComment, cardChange, cardOps, commentFreeText, formToOps, isSplit, reviewFormOf, similarOps} from "./editMapping";
import type {WireEditOp} from "./native";

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

function leg(account: string, cents: bigint | null, extra: Partial<Posting> = {}): Posting {
    return {
        account,
        amounts: cents === null ? [] : [{commodity: "$", qty: {m: cents, p: 2}, style: STYLE}],
        status: "unmarked",
        comment: "",
        tags: [],
        ...extra,
    };
}

/** `SQ *BLUE BOTTLE 4412 OAKLAND  ; note, trip:sf` paying $6.75 from the Visa, elided. */
const CARD: Transaction = {
    index: 12,
    date: "2026-10-12",
    status: "unmarked",
    description: "SQ *BLUE BOTTLE 4412 OAKLAND",
    code: "",
    comment: "note, trip:sf\nreceipt: 0042",
    tags: [
        ["trip", "sf"],
        ["receipt", "0042"],
    ],
    postings: [leg("expenses:unknown", 675n), leg("liabilities:visa", null)],
};

const form = (edit: (form: ReviewForm) => void = () => {}): ReviewForm => {
    const draft = reviewFormOf(CARD);
    edit(draft);
    return draft;
};

describe("UNIT review form", () => {
    it("reads the free text, the tag chips and the written postings", () => {
        const draft = reviewFormOf(CARD);
        expect(draft.comment).toBe("note");
        expect(draft.tags).toEqual([
            ["trip", "sf"],
            ["receipt", "0042"],
        ]);
        expect(draft.postings.map((p) => [p.account, p.amount])).toEqual([
            ["expenses:unknown", "6.75"],
            ["liabilities:visa", ""],
        ]);
    });

    it("leaves generated postings and machine tags off the card", () => {
        const auto: Transaction = {
            ...CARD,
            tags: [...CARD.tags, ["modified-transaction", ""], ["_modified-transaction", ""]],
            postings: [...CARD.postings, leg("(budget:unknown)", 675n, {generated: {rule: 0, query: "expenses"}})],
        };
        const draft = reviewFormOf(auto);
        expect(draft.postings).toHaveLength(2);
        expect(draft.tags.map(([name]) => name)).toEqual(["trip", "receipt"]);
    });

    it.each([
        ["", ""],
        ["groceries", "groceries"],
        ["groceries, reviewed:yes", "groceries"],
        ["reviewed:2026-01-02, x:1", ""],
        ["weekly shop: produce", ""],
        ["a, b, c:1", "a, b"],
        ["first\nsecond line", "first\nsecond line"],
        ["\nown line, id:7", "own line"],
        ["a\nreviewed:x\n\nb, c:1", "a\nb"],
    ])("free text of %j is %j", (comment, free) => {
        expect(commentFreeText(comment)).toBe(free);
    });
});

describe("UNIT formToOps", () => {
    it("is empty when nothing changed", () => {
        expect(formToOps(CARD, form())).toEqual([]);
        expect(isSplit(CARD, form())).toBe(false);
    });

    it.each<[string, (draft: ReviewForm) => void, unknown[]]>([
        ["the description", (d) => (d.description = "Blue Bottle Coffee"), [{op: "setDescription", description: "Blue Bottle Coffee"}]],
        [
            "an account",
            (d) => ((d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee"),
            [{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}],
        ],
        ["a tag's value", (d) => (d.tags[0] = ["trip", "la"]), [{op: "setTxnTag", name: "trip", value: "la"}]],
        ["a new tag", (d) => d.tags.push(["reviewed", "2026-10-30"]), [{op: "setTxnTag", name: "reviewed", value: "2026-10-30"}]],
        ["a removed tag", (d) => d.tags.splice(0, 1), [{op: "setTxnTag", name: "trip", value: null}]],
        ["the comment", (d) => (d.comment = "birthday"), [{op: "setTxnComment", comment: "birthday"}]],
        ["the comment, cleared", (d) => (d.comment = ""), [{op: "setTxnComment", comment: ""}]],
    ])("writes %s surgically", (_, edit, ops) => {
        const draft = form(edit);
        expect(isSplit(CARD, draft)).toBe(false);
        expect(formToOps(CARD, draft)).toEqual(ops);
    });

    it("combines surgical ops in engine order: description, accounts, tags, comment", () => {
        const draft = form((d) => {
            d.comment = "gift";
            d.tags.push(["reviewed", "2026-10-30"]);
            (d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee";
            d.description = "Blue Bottle Coffee";
        });
        expect(formToOps(CARD, draft).map((op) => op.op)).toEqual(["setDescription", "setPostingAccount", "setTxnTag", "setTxnComment"]);
    });

    it("never touches the elided posting, and an amount retyped to the same value is no change", () => {
        const draft = form((d) => {
            (d.postings[0] as ReviewForm["postings"][number]).amount = "6.750";
            (d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee";
        });
        expect(isSplit(CARD, draft)).toBe(false);
        expect(formToOps(CARD, draft)).toEqual([{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}]);
    });

    it.each<[string, (draft: ReviewForm) => void]>([
        ["an amount", (d) => ((d.postings[0] as ReviewForm["postings"][number]).amount = "7.00")],
        ["the elided leg given an amount", (d) => ((d.postings[1] as ReviewForm["postings"][number]).amount = "-6.75")],
        ["a posting added", (d) => d.postings.push({...(d.postings[0] as ReviewForm["postings"][number]), account: "expenses:tips", amount: "1.00"})],
        ["a posting removed", (d) => d.postings.splice(1, 1)],
        ["a commodity", (d) => ((d.postings[0] as ReviewForm["postings"][number]).commodity = "EUR")],
    ])("replaces for a split: %s", (_, edit) => {
        const draft = form(edit);
        expect(isSplit(CARD, draft)).toBe(true);
        const ops = formToOps(CARD, draft);
        expect(ops[0]?.op).toBe("replace");
        expect(ops.slice(1)).toEqual([]);
        const replace = ops[0] as Extract<(typeof ops)[number], {op: "replace"}>;
        expect(replace.transaction.comment).toBe(CARD.comment.trim());
    });

    it("writes a date change surgically (D34), then tags and comment on top", () => {
        const draft = form((d) => {
            d.date = "2026-10-11";
            d.comment = "moved";
            d.tags.push(["reviewed", "2026-10-30"]);
        });
        expect(isSplit(CARD, draft)).toBe(false);
        expect(formToOps(CARD, draft)).toEqual([
            {op: "setDate", date: "2026-10-11"},
            {op: "setTxnTag", name: "reviewed", value: "2026-10-30"},
            {op: "setTxnComment", comment: "moved"},
        ]);
    });
});

/**
 * Every field of the form → its op (plan 32 D34), over a transaction that has
 * all of them: a secondary date, a status, a code, a two-line comment, a
 * commented posting with an assertion, and an elided leg AS THE ENGINE SERVES
 * IT (the inferred amount present, marked `elided`).
 */
describe("UNIT formToOps — one op per field", () => {
    const FULL: Transaction = {
        index: 7,
        date: "2026-10-12",
        date2: "2026-10-14",
        status: "cleared",
        description: "Blue Bottle",
        code: "1042",
        comment: "split with Sam, trip:sf\nsecond line",
        tags: [["trip", "sf"]],
        postings: [
            leg("expenses:food:coffee", 675n, {
                comment: "gift card",
                balanceAssertion: {amount: {commodity: "$", qty: {m: 675n, p: 2}, style: STYLE}, inclusive: false, total: false},
            }),
            leg("liabilities:visa", -675n, {elided: true}),
        ],
    };
    const edited = (edit: (form: ReviewForm) => void): ReviewForm => {
        const draft = reviewFormOf(FULL);
        edit(draft);
        return draft;
    };
    const row = (d: ReviewForm, at: number): ReviewForm["postings"][number] => d.postings[at] as ReviewForm["postings"][number];

    it("reads every field, and leaves the elided leg blank", () => {
        const draft = reviewFormOf(FULL);
        expect([draft.date, draft.date2, draft.status, draft.code]).toEqual(["2026-10-12", "2026-10-14", "cleared", "1042"]);
        expect(draft.comment).toBe("split with Sam\nsecond line");
        expect(draft.postings.map((p) => [p.amount, p.elided ?? false, p.inferred ?? null])).toEqual([
            ["6.75", false, null],
            ["", true, "-6.75"],
        ]);
        expect(formToOps(FULL, draft)).toEqual([]);
    });

    it.each<[string, (draft: ReviewForm) => void, unknown[]]>([
        ["the date", (d) => (d.date = "2026-10-11"), [{op: "setDate", date: "2026-10-11"}]],
        ["the secondary date", (d) => (d.date2 = "2026-10-15"), [{op: "setDate2", date2: "2026-10-15"}]],
        ["the secondary date, cleared", (d) => (d.date2 = " "), [{op: "setDate2", date2: null}]],
        ["the status", (d) => (d.status = "pending"), [{op: "setStatus", status: "pending"}]],
        ["the code", (d) => (d.code = " INV 9 "), [{op: "setCode", code: "INV 9"}]],
        ["the code, cleared", (d) => (d.code = ""), [{op: "setCode", code: null}]],
        ["the description", (d) => (d.description = "Blue Bottle Coffee"), [{op: "setDescription", description: "Blue Bottle Coffee"}]],
        ["an account", (d) => (row(d, 1).account = "liabilities:amex"), [{op: "setPostingAccount", posting: 1, account: "liabilities:amex"}]],
        ["a posting comment", (d) => (row(d, 0).comment = "gift card\nask Sam  "), [{op: "setPostingComment", posting: 0, comment: "gift card\nask Sam"}]],
        ["a posting comment, added on the elided leg", (d) => (row(d, 1).comment = "paid"), [{op: "setPostingComment", posting: 1, comment: "paid"}]],
        ["a posting comment, cleared", (d) => (row(d, 0).comment = "  "), [{op: "setPostingComment", posting: 0, comment: null}]],
        ["an assertion, removed", (d) => (row(d, 0).balanceAssertion = null), [{op: "removePostingAssertion", posting: 0}]],
        ["a tag", (d) => (d.tags = [["trip", "la"]]), [{op: "setTxnTag", name: "trip", value: "la"}]],
        ["the comment, a line edited", (d) => (d.comment = "split with Alex\nsecond line"), [{op: "setTxnComment", comment: "split with Alex\nsecond line"}]],
        [
            "the comment, a line added",
            (d) => (d.comment = "split with Sam\r\nsecond line\n\nthird\n"),
            [{op: "setTxnComment", comment: "split with Sam\nsecond line\nthird"}],
        ],
        ["the comment, cleared", (d) => (d.comment = ""), [{op: "setTxnComment", comment: ""}]],
    ])("writes %s surgically", (_, edit, ops) => {
        const draft = edited(edit);
        expect(isSplit(FULL, draft)).toBe(false);
        expect(formToOps(FULL, draft)).toEqual(ops);
    });

    it("leaves the comment alone when only its blank lines or spacing differ", () => {
        expect(
            formToOps(
                FULL,
                edited((d) => (d.comment = "  split with Sam \n\n second line\n"))
            )
        ).toEqual([]);
        const commas: Transaction = {...FULL, comment: "a, b, trip:sf"};
        const draft = reviewFormOf(commas);
        draft.comment = "a,b";
        expect(formToOps(commas, draft)).toEqual([]);
        draft.comment = "a,,b, c";
        expect(formToOps(commas, draft)).toEqual([{op: "setTxnComment", comment: "a, b, c"}]);
    });

    it("puts every header op before the posting ops, in one entry", () => {
        const draft = edited((d) => {
            row(d, 0).comment = "";
            d.description = "BB";
            d.code = "";
            d.status = "unmarked";
            d.date2 = "";
            d.date = "2026-10-10";
        });
        expect(formToOps(FULL, draft).map((op) => op.op)).toEqual(["setDate", "setDate2", "setStatus", "setCode", "setDescription", "setPostingComment"]);
    });

    it.each<[string, (draft: ReviewForm) => void]>([
        [
            "an assertion changed",
            (d) => (row(d, 0).balanceAssertion = {amount: {commodity: "$", quantity: {mantissa: "700", places: 2}}, inclusive: false, total: false}),
        ],
        [
            "an assertion added",
            (d) => (row(d, 1).balanceAssertion = {amount: {commodity: "$", quantity: {mantissa: "0", places: 2}}, inclusive: false, total: true}),
        ],
        ["a posting's status", (d) => (row(d, 0).status = "pending")],
        ["the elided leg typed", (d) => (row(d, 1).amount = "-6.75")],
    ])("replaces for %s", (_, edit) => {
        expect(isSplit(FULL, edited(edit))).toBe(true);
        expect(formToOps(FULL, edited(edit))[0]?.op).toBe("replace");
    });

    it("re-renders from the form's header, keeps the untouched elided leg blank, and writes a typed one", () => {
        const draft = edited((d) => {
            row(d, 0).amount = "7.25";
            row(d, 0).comment = "\nown line";
            d.status = "pending";
            d.code = "";
            d.date2 = "2026-10-20";
        });
        const ops = formToOps(FULL, draft);
        expect(ops.map((op) => op.op)).toEqual(["replace"]);
        const replace = ops[0] as Extract<(typeof ops)[number], {op: "replace"}>;
        expect(replace.transaction).toEqual({
            date: "2026-10-12",
            date2: "2026-10-20",
            status: "pending",
            description: "Blue Bottle",
            comment: "split with Sam, trip:sf\nsecond line",
            postings: [
                {
                    account: "expenses:food:coffee",
                    // A comment on its own line keeps its leading line break.
                    comment: "\nown line",
                    amount: {commodity: "$", quantity: {mantissa: "725", places: 2}},
                    balanceAssertion: {amount: {commodity: "$", quantity: {mantissa: "675", places: 2}}, inclusive: false, total: false},
                },
                // Untouched: written blank, so it stays auto-balancing (D35).
                {account: "liabilities:visa"},
            ],
        });
        const typed = formToOps(
            FULL,
            edited((d) => {
                row(d, 0).amount = "7.25";
                row(d, 1).amount = "-7.25";
            })
        )[0] as Extract<WireEditOp, {op: "replace"}>;
        expect(typed.transaction.postings[1]).toEqual({account: "liabilities:visa", amount: {commodity: "$", quantity: {mantissa: "-725", places: 2}}});
    });
});

describe("UNIT cardOps", () => {
    it("adds the reviewed stamp, and payee-key only on a rename", () => {
        expect(cardOps(CARD, form(), {key: "sq blue bottle oakland", reviewedOn: "2026-10-30"})).toEqual([
            {op: "setTxnTag", name: "reviewed", value: "2026-10-30"},
        ]);
        const renamed = form((d) => (d.description = "Blue Bottle Coffee"));
        expect(cardOps(CARD, renamed, {key: "sq blue bottle oakland", reviewedOn: null})).toEqual([
            {op: "setDescription", description: "Blue Bottle Coffee"},
            {op: "setTxnTag", name: "payee-key", value: "sq blue bottle oakland"},
        ]);
        // No key to record, or one already recorded: no payee-key.
        expect(cardOps(CARD, renamed, {key: null, reviewedOn: null})).toHaveLength(1);
        const keyed = {...CARD, tags: [...CARD.tags, ["payee-key", "x"]] as [string, string][]};
        expect(cardOps(keyed, reviewFormOf(keyed), {key: "y", reviewedOn: null})).toEqual([]);
    });
});

describe("UNIT similar apply", () => {
    const sibling: Transaction = {...CARD, index: 9, description: "SQ *BLUE BOTTLE 0917 OAKLAND", comment: "", tags: []};

    it("moves every written posting on a moved account and renames", () => {
        const change = cardChange(
            CARD,
            form((d) => {
                d.description = "Blue Bottle Coffee";
                (d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee";
            })
        );
        expect(change).not.toBeNull();
        const withGenerated: Transaction = {
            ...sibling,
            postings: [leg("(budget:x)", 1n, {generated: {rule: 0, query: "q"}}), ...sibling.postings, leg("expenses:unknown", 1n)],
        };
        expect(similarOps(withGenerated, change!)).toEqual([
            {op: "setDescription", description: "Blue Bottle Coffee"},
            // Written positions: the generated posting is not counted.
            {op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"},
            {op: "setPostingAccount", posting: 2, account: "expenses:food:coffee"},
        ]);
    });

    it("offers nothing for a split, and nothing a candidate cannot take", () => {
        expect(
            cardChange(
                CARD,
                form((d) => ((d.postings[0] as ReviewForm["postings"][number]).amount = "1.00"))
            )
        ).toBeNull();
        expect(
            cardChange(
                CARD,
                form((d) => (d.comment = "only a comment"))
            )
        ).toBeNull();
        // A posting comment or a date is not a split: the account change still applies.
        expect(
            cardChange(
                CARD,
                form((d) => {
                    d.date = "2026-10-01";
                    (d.postings[0] as ReviewForm["postings"][number]).comment = "noted";
                    (d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee";
                })
            )
        ).not.toBeNull();
        const change = cardChange(
            CARD,
            form((d) => ((d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee"))
        );
        const elsewhere: Transaction = {...sibling, postings: [leg("expenses:food:groceries", 100n), leg("assets:cash", null)]};
        expect(similarOps(elsewhere, change!)).toEqual([]);
    });
});

describe("UNIT editMapping — addComment (plan 32 D36)", () => {
    const form = (comment: string, tags: [string, string][]): ReviewForm => ({
        date: "2026-10-30",
        date2: "",
        status: "unmarked",
        code: "",
        description: "Netflix",
        comment,
        tags,
        postings: [],
    });

    it("puts the free text, then each tag, on the header's comment line", () => {
        expect(addComment(form("family plan", [["reviewed", "2026-10-30"]]))).toBe("family plan, reviewed:2026-10-30");
    });

    it("writes tags alone when there is no free text, and nothing when there is neither", () => {
        expect(
            addComment(
                form("", [
                    ["receipt", "0042"],
                    ["reviewed", "ignore"],
                ])
            )
        ).toBe("receipt:0042, reviewed:ignore");
        expect(addComment(form("  ", []))).toBe("");
    });

    it("keeps later free-text lines on lines of their own", () => {
        expect(addComment(form("one\n\n two ,three", [["trip", "nyc"]]))).toBe("one, trip:nyc\ntwo, three");
    });
});
