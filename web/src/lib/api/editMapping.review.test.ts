// The review card's save (plan 32 D9, D11): which ops a form becomes. The
// engine applies them (crates/ledgeline-core/tests/edit_batch.rs); what is
// pinned here is the CHOICE — surgical where it can be, `replace` only when a
// split or a date forces it — because a needless `replace` reformats the
// user's text and loses an elided amount.

import {describe, expect, it} from "vitest";
import type {AmountStyle, Posting, Transaction} from "#lib/domain/types.js";
import type {ReviewForm} from "#lib/review/types.js";
import {cardChange, cardOps, commentFreeText, formToOps, isSplit, reviewFormOf, similarOps} from "./editMapping";

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
        ["first\nsecond line", "first"],
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

    it("replaces for a date change, which is not a split, then writes tags and comment on top", () => {
        const draft = form((d) => {
            d.date = "2026-10-11";
            d.comment = "moved";
            d.tags.push(["reviewed", "2026-10-30"]);
        });
        expect(isSplit(CARD, draft)).toBe(false);
        const ops = formToOps(CARD, draft);
        expect(ops.map((op) => op.op)).toEqual(["replace", "setTxnTag", "setTxnComment"]);
        const replace = ops[0] as Extract<(typeof ops)[number], {op: "replace"}>;
        expect(replace.transaction.date).toBe("2026-10-11");
        // The elided leg stays elided in the re-render: no amount sent.
        expect(replace.transaction.postings[1]).toEqual({account: "liabilities:visa"});
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
        const change = cardChange(
            CARD,
            form((d) => ((d.postings[0] as ReviewForm["postings"][number]).account = "expenses:food:coffee"))
        );
        const elsewhere: Transaction = {...sibling, postings: [leg("expenses:food:groceries", 100n), leg("assets:cash", null)]};
        expect(similarOps(elsewhere, change!)).toEqual([]);
    });
});
