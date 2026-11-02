// The complete review card (plan 32 Phase 7b, D33–D35): every field the old
// TransactionModal could edit, edited on the card, and the exact batch each
// sends (the parity matrix); the details row collapsed and expanded; its keys
// (`D`, `!`, `#`); the comment textarea that keeps continuation lines (p7
// amendment 17); and the elided leg's placeholder (amendment 18). Over an
// import the same edits reach the import's sink.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {decodeCsvStagedTxn} from "#lib/api/importReviewDecode.js";
import type {Amount, Posting, Transaction} from "#lib/domain/types.js";
import type {StagedSave} from "#lib/imports/reviewTypes.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {connectFakeEngine} from "#lib/testing/fakeEngine.js";
import {queueItemWire, queuePageWire, reviewTxnWire, serveReview, stagedTxnWire} from "#lib/testing/reviewFixture.js";
import {accountField, card, MOD, mountOverlay, press, settle} from "#lib/testing/reviewOverlay.js";
import {resetReviewLaunch, reviewLaunch} from "./launch.svelte";
import ReviewOverlay from "./ReviewOverlay.svelte";

const TODAY = localToday();
const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null} as const;
const money = (cents: bigint, commodity = "$"): Amount => ({commodity, qty: {m: cents, p: 2}, style: STYLE});
const leg = (account: string, amount: Amount | null, extra: Partial<Posting> = {}): Posting => ({
    account,
    amounts: amount === null ? [] : [amount],
    status: "unmarked",
    comment: "",
    tags: [],
    ...extra,
});

/** An unknown coffee, the card leg written, the card's leg elided (the file writes it blank: `liabilities:visa`). */
function coffee(extra: Partial<Transaction> = {}, postings?: Posting[]): Transaction {
    return {
        index: 1,
        date: "2026-10-12",
        status: "unmarked",
        description: "BLUE BOTTLE",
        code: "",
        comment: "",
        tags: [],
        postings: postings ?? [leg("expenses:unknown", money(675n)), leg("liabilities:visa", money(-675n), {elided: true})],
        ...extra,
    };
}

/** The same with every leg written, the second asserting the card's balance (`== $0.00`). */
const asserted = (): Transaction =>
    coffee({}, [
        leg("expenses:unknown", money(675n)),
        leg("liabilities:visa", money(-675n), {balanceAssertion: {amount: money(0n), inclusive: false, total: true}}),
    ]);

type Body = {edits: {index: number; fingerprint: string; ops: unknown[]}[]};

/** A one-card Unknowns run over `txn`; every batch body is kept. */
function serve(txn: Transaction): Body[] {
    const bodies: Body[] = [];
    serveReview({
        queue: () => queuePageWire("unknowns", [queueItemWire(txn)]),
        batch: (body) => {
            bodies.push(body as Body);
            return {generation: "g2", edited: body.edits.map(() => reviewTxnWire(txn, {fingerprint: "fp-1-1"}))};
        },
    });
    return bodies;
}

/** ⌘↵ from the card, then the write. */
async function save(): Promise<void> {
    card().focus();
    await press("Enter", MOD);
    await review.settled();
}

/** The one edit a save of card 1 sent. */
const sent = (ops: unknown[]): Body[] => [{edits: [{index: 1, fingerprint: "fp-1-0", ops}]}];

const input = (label: string): HTMLInputElement => screen.getByLabelText(label) as HTMLInputElement;
const type = async (label: string, value: string): Promise<void> => void (await fireEvent.input(input(label), {target: {value}}));
const usd = (mantissa: string) => ({commodity: "$", quantity: {mantissa, places: 2}});
const REVIEWED = {op: "setTxnTag", name: "reviewed", value: TODAY};

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    await connectFakeEngine();
});
afterEach(() => {
    vi.unstubAllGlobals();
    keymap.reset();
});

interface ParityCase {
    name: string;
    txn: () => Transaction;
    edit: () => Promise<void>;
    ops: unknown[];
}

/**
 * Each capability of the old `TransactionModal`, edited on the card. The
 * unknown leg stays unknown (except where the account is the edit), so a save
 * writes the edit alone, without `reviewed:` (D7's dirty-unknown).
 */
const PARITY: ParityCase[] = [
    {
        name: "date",
        txn: coffee,
        edit: async () => {
            await press("D");
            await type("Date", "2026-10-13");
        },
        ops: [{op: "setDate", date: "2026-10-13"}],
    },
    {
        name: "secondary date, added",
        txn: coffee,
        edit: async () => {
            await press("D");
            await fireEvent.click(screen.getByRole("button", {name: "+ secondary date"}));
            await type("Secondary date", "2026-10-14");
        },
        ops: [{op: "setDate2", date2: "2026-10-14"}],
    },
    {
        name: "secondary date, edited",
        txn: () => coffee({date2: "2026-10-14"}),
        edit: async () => {
            await press("D");
            await type("Secondary date", "2026-10-15");
        },
        ops: [{op: "setDate2", date2: "2026-10-15"}],
    },
    {
        name: "secondary date, removed",
        txn: () => coffee({date2: "2026-10-14"}),
        edit: async () => {
            await press("D");
            await fireEvent.click(screen.getByRole("button", {name: "Remove the secondary date"}));
        },
        ops: [{op: "setDate2", date2: null}],
    },
    {
        name: "status",
        txn: coffee,
        edit: async () => {
            await press("!");
            await press("!");
        },
        ops: [{op: "setStatus", status: "cleared"}],
    },
    {
        name: "code, set",
        txn: coffee,
        edit: async () => {
            await press("#");
            await type("Code", "1042");
        },
        ops: [{op: "setCode", code: "1042"}],
    },
    {
        name: "code, removed",
        txn: () => coffee({code: "1042"}),
        edit: async () => {
            await press("#");
            await type("Code", "");
        },
        ops: [{op: "setCode", code: null}],
    },
    {
        name: "description",
        txn: coffee,
        edit: () => type("Description", "Blue Bottle Coffee"),
        ops: [
            {op: "setDescription", description: "Blue Bottle Coffee"},
            {op: "setTxnTag", name: "payee-key", value: "payee"},
        ],
    },
    {
        name: "comment, over two lines",
        txn: coffee,
        edit: () => type("Comment", "split with Sam\nreceipt in the drawer"),
        ops: [{op: "setTxnComment", comment: "split with Sam\nreceipt in the drawer"}],
    },
    {
        name: "tags",
        txn: coffee,
        edit: async () => {
            await type("Add a tag", "receipt:42");
            await fireEvent.keyDown(input("Add a tag"), {key: "Enter"});
        },
        ops: [{op: "setTxnTag", name: "receipt", value: "42"}],
    },
    {
        name: "posting account",
        txn: coffee,
        edit: () => type("Account for posting 1", "expenses:food:coffee"),
        ops: [{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}, REVIEWED],
    },
    {
        name: "posting amount (a split: the elided leg stays blank)",
        txn: coffee,
        edit: () => type("Amount for posting 1", "7.25"),
        ops: [
            {
                op: "replace",
                transaction: {
                    date: "2026-10-12",
                    description: "BLUE BOTTLE",
                    postings: [{account: "expenses:unknown", amount: usd("725")}, {account: "liabilities:visa"}],
                },
            },
        ],
    },
    {
        name: "posting commodity",
        txn: coffee,
        edit: () => type("Commodity for posting 1", "EUR"),
        ops: [
            {
                op: "replace",
                transaction: {
                    date: "2026-10-12",
                    description: "BLUE BOTTLE",
                    postings: [
                        {account: "expenses:unknown", amount: {commodity: "EUR", quantity: {mantissa: "675", places: 2}}},
                        {account: "liabilities:visa"},
                    ],
                },
            },
        ],
    },
    {
        name: "posting comment, added from ⋯",
        txn: coffee,
        edit: async () => {
            await fireEvent.click(screen.getByRole("button", {name: "Add a comment to posting 1"}));
            await type("Comment for posting 1", "paid with the gift card");
        },
        ops: [{op: "setPostingComment", posting: 0, comment: "paid with the gift card"}],
    },
    {
        name: "posting comment, edited in place over two lines",
        txn: () => coffee({}, [leg("expenses:unknown", money(675n), {comment: "gift card"}), leg("liabilities:visa", money(-675n), {elided: true})]),
        edit: () => type("Comment for posting 1", "gift card\nfrom Sam"),
        ops: [{op: "setPostingComment", posting: 0, comment: "gift card\nfrom Sam"}],
    },
    {
        name: "posting comment, cleared",
        txn: () => coffee({}, [leg("expenses:unknown", money(675n), {comment: "gift card"}), leg("liabilities:visa", money(-675n), {elided: true})]),
        edit: () => type("Comment for posting 1", ""),
        ops: [{op: "setPostingComment", posting: 0, comment: null}],
    },
    {
        name: "add a posting",
        txn: coffee,
        edit: async () => {
            await fireEvent.click(screen.getByRole("button", {name: /Add posting/}));
            await type("Account for posting 3", "assets:cash");
            await type("Amount for posting 3", "1.00");
        },
        ops: [
            {
                op: "replace",
                transaction: {
                    date: "2026-10-12",
                    description: "BLUE BOTTLE",
                    postings: [{account: "expenses:unknown", amount: usd("675")}, {account: "liabilities:visa"}, {account: "assets:cash", amount: usd("100")}],
                },
            },
        ],
    },
    {
        name: "remove a posting",
        txn: () =>
            coffee({}, [leg("expenses:unknown", money(675n)), leg("expenses:food:tip", money(100n)), leg("liabilities:visa", money(-775n), {elided: true})]),
        edit: () => fireEvent.click(screen.getByRole("button", {name: "Remove posting 2"})).then(() => undefined),
        ops: [
            {
                op: "replace",
                transaction: {
                    date: "2026-10-12",
                    description: "BLUE BOTTLE",
                    postings: [{account: "expenses:unknown", amount: usd("675")}, {account: "liabilities:visa"}],
                },
            },
        ],
    },
    {
        name: "remove a balance assertion",
        txn: asserted,
        edit: () => fireEvent.click(screen.getByRole("button", {name: "Remove the balance assertion on posting 2"})).then(() => undefined),
        ops: [{op: "removePostingAssertion", posting: 1}],
    },
];

describe("COMPONENT ReviewOverlay — the complete card (parity with the old editor)", () => {
    it.each(PARITY)("$name: sends exactly its op", async ({txn, edit, ops}) => {
        const bodies = serve(txn());
        await mountOverlay();
        card().focus();
        await edit();
        await save();
        expect(bodies).toEqual(sent(ops));
    });

    it("sends several header and posting edits as one surgical batch, in the engine's order", async () => {
        const bodies = serve(asserted());
        await mountOverlay();
        card().focus();
        await press("#");
        await type("Code", "7");
        card().focus();
        await press("!");
        await fireEvent.click(screen.getByRole("button", {name: "Add a comment to posting 2"}));
        await type("Comment for posting 2", "statement 10/12");
        await fireEvent.click(screen.getByRole("button", {name: "Remove the balance assertion on posting 2"}));
        await type("Comment", "a note");
        await save();
        expect(bodies).toEqual(
            sent([
                {op: "setStatus", status: "pending"},
                {op: "setCode", code: "7"},
                {op: "setPostingComment", posting: 1, comment: "statement 10/12"},
                {op: "removePostingAssertion", posting: 1},
                {op: "setTxnComment", comment: "a note"},
            ])
        );
    });
});

describe("COMPONENT ReviewOverlay — the comment keeps its lines (p7 amendment 17)", () => {
    const twoLines = (): Transaction => coffee({comment: "first note, id:7\nsecond note"});

    it("is a textarea holding every line of the free text", async () => {
        serve(twoLines());
        await mountOverlay();
        const box = input("Comment");
        expect(box.tagName).toBe("TEXTAREA");
        expect(box.value).toBe("first note\nsecond note");
    });

    it("sends no comment op when it is left alone", async () => {
        const bodies = serve(twoLines());
        await mountOverlay();
        await type("Account for posting 1", "expenses:food:coffee");
        await save();
        expect(bodies).toEqual(sent([{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}, REVIEWED]));
    });

    it("sends the whole free text when the second line changes", async () => {
        const bodies = serve(twoLines());
        await mountOverlay();
        await type("Comment", "first note\nsecond note, revised");
        await save();
        expect(bodies).toEqual(sent([{op: "setTxnComment", comment: "first note\nsecond note, revised"}]));
    });

    it("types a newline on Enter, and still saves on ⌘↵ and leaves on Escape", async () => {
        const bodies = serve(twoLines());
        await mountOverlay();
        const box = input("Comment");
        box.focus();
        const enter = await press("Enter");
        expect(enter.defaultPrevented).toBe(false);
        expect(bodies).toEqual([]);
        await press("Escape");
        expect(document.activeElement).toBe(card());
        await type("Comment", "first note\nsecond note\nthird");
        input("Comment").focus();
        await press("Enter", MOD);
        await review.settled();
        expect(bodies).toEqual(sent([{op: "setTxnComment", comment: "first note\nsecond note\nthird"}]));
    });

    it("is reached by m", async () => {
        serve(twoLines());
        await mountOverlay();
        card().focus();
        await press("m");
        expect(document.activeElement).toBe(input("Comment"));
    });
});

describe("COMPONENT ReviewOverlay — the details row (D33)", () => {
    const rich = (): Transaction => coffee({status: "cleared", code: "1042", date2: "2026-10-14"});
    const summary = (): HTMLElement => screen.getByTestId("review-details-summary");
    /** The line as read: the separators are drawn apart by the layout's gap, not by spaces. */
    const text = (element: HTMLElement): string =>
        (element.textContent ?? "")
            .replace(/\s*·\s*/g, " · ")
            .replace(/\s+/g, " ")
            .trim();

    it("starts collapsed to one line in the hero: date, secondary date, status, code", async () => {
        serve(rich());
        await mountOverlay();
        expect(text(summary())).toBe("Oct 12, 2026 · secondary Oct 14, 2026 · cleared · #1042");
        expect(within(screen.getByTestId("review-hero")).getByTestId("review-details-summary")).toBeTruthy();
        expect(screen.queryByTestId("review-details")).toBeNull();
        expect(screen.queryByLabelText("Date")).toBeNull();
    });

    it("shows just the date when nothing else is set", async () => {
        serve(coffee());
        await mountOverlay();
        expect(text(summary())).toBe("Oct 12, 2026");
    });

    it("expands on a click, with the date focused, and folds back while nothing in it changed", async () => {
        serve(rich());
        await mountOverlay();
        await fireEvent.click(summary());
        await settle();
        const details = screen.getByTestId("review-details");
        expect(document.activeElement).toBe(input("Date"));
        expect(input("Date").value).toBe("2026-10-12");
        expect(input("Secondary date").value).toBe("2026-10-14");
        expect(input("Code").value).toBe("1042");
        expect(within(details).getByTestId("review-status").dataset.status).toBe("cleared");
        await fireEvent.click(screen.getByRole("button", {name: "Hide the details"}));
        expect(screen.queryByTestId("review-details")).toBeNull();
        expect(document.activeElement).toBe(card());
    });

    it("stays expanded while a field in it is changed, and the line then shows the edit", async () => {
        serve(coffee());
        await mountOverlay();
        await press("#");
        await type("Code", "88");
        expect(screen.queryByRole("button", {name: "Hide the details"})).toBeNull();
        await type("Code", "");
        await fireEvent.click(screen.getByRole("button", {name: "Hide the details"}));
        expect(text(summary())).toBe("Oct 12, 2026");
    });

    it("folds back on the next card, and on Revert", async () => {
        serve(coffee());
        await mountOverlay();
        await press("!");
        expect(screen.getByTestId("review-details")).toBeTruthy();
        card().focus();
        await press("z");
        expect(screen.queryByTestId("review-details")).toBeNull();
        expect(text(summary())).toBe("Oct 12, 2026");
    });

    it("cycles the status pill on a click, as the Journal's status cell does", async () => {
        serve(coffee());
        await mountOverlay();
        await press("D");
        const pill = (): HTMLElement => screen.getByTestId("review-status");
        expect(pill().dataset.status).toBe("unmarked");
        await fireEvent.click(pill());
        expect(pill().dataset.status).toBe("pending");
        await fireEvent.click(pill());
        expect(pill().dataset.status).toBe("cleared");
        await fireEvent.click(pill());
        expect(pill().dataset.status).toBe("unmarked");
    });

    it("adds a secondary date from ⋯, and the menu no longer offers it once the card has one", async () => {
        serve(coffee());
        await mountOverlay();
        await fireEvent.click(screen.getByRole("button", {name: /Add secondary date/}));
        await settle();
        expect(document.activeElement).toBe(input("Secondary date"));
        expect(screen.queryByRole("button", {name: /Add secondary date/})).toBeNull();
        expect(screen.queryByRole("button", {name: /Edit date/})).toBeNull();
    });

    it("shows an unset secondary date as unset, not as today (WebKit)", async () => {
        serve(coffee());
        await mountOverlay();
        await press("D");
        await fireEvent.click(screen.getByRole("button", {name: "+ secondary date"}));
        expect(input("Secondary date").value).toBe("");
        expect(within(screen.getByTestId("review-details")).getByTestId("date-unset")).toBeTruthy();
    });
});

describe("COMPONENT ReviewOverlay — details keys (D40)", () => {
    it("D opens the details on the date, # on the code, and ! cycles the status, in card mode only", async () => {
        serve(coffee());
        await mountOverlay();
        card().focus();
        await press("D");
        expect(document.activeElement).toBe(input("Date"));
        card().focus();
        await press("#");
        expect(document.activeElement).toBe(input("Code"));
        card().focus();
        await press("!");
        expect(screen.getByTestId("review-status").dataset.status).toBe("pending");
        // In a field, they type.
        input("Code").focus();
        const typed = await press("!");
        expect(typed.defaultPrevented).toBe(false);
        expect(screen.getByTestId("review-status").dataset.status).toBe("pending");
    });

    it("lists them in the ? sheet beside the existing card keys", async () => {
        serve(coffee());
        await mountOverlay();
        const help = JSON.stringify(keymap.help);
        for (const label of ["Edit the date", "Cycle the status", "Edit the code", "Edit description", "Open or close Similar"]) expect(help).toContain(label);
    });

    it("leaves d on the description and $ on the amount, not its commodity", async () => {
        serve(coffee());
        await mountOverlay();
        card().focus();
        await press("d");
        expect(document.activeElement).toBe(input("Description"));
        card().focus();
        await press("$");
        expect(document.activeElement).toBe(input("Amount for posting 1"));
    });
});

describe("COMPONENT ReviewOverlay — postings (D33, D35)", () => {
    it("shows an elided leg's balancing value as its placeholder, blank, and sends nothing for it", async () => {
        const bodies = serve(coffee());
        await mountOverlay();
        const amount = input("Amount for posting 2");
        expect(amount.value).toBe("");
        expect(amount.placeholder).toBe("auto · -6.75");
        expect(input("Amount for posting 1").placeholder).toBe("auto");
        await type("Account for posting 1", "expenses:food:coffee");
        await save();
        expect(bodies).toEqual(sent([{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}, REVIEWED]));
    });

    it("keeps the placeholder true while the other amount is edited", async () => {
        serve(coffee());
        await mountOverlay();
        await type("Amount for posting 1", "7.25");
        expect(input("Amount for posting 2").placeholder).toBe("auto · -7.25");
        expect(input("Amount for posting 2").value).toBe("");
    });

    it("writes an elided leg out once a value is typed in it", async () => {
        const bodies = serve(coffee());
        await mountOverlay();
        await type("Amount for posting 2", "-6.75");
        await save();
        expect(bodies).toEqual(
            sent([
                {
                    op: "replace",
                    transaction: {
                        date: "2026-10-12",
                        description: "BLUE BOTTLE",
                        postings: [
                            {account: "expenses:unknown", amount: usd("675")},
                            {account: "liabilities:visa", amount: usd("-675")},
                        ],
                    },
                },
            ])
        );
    });

    it("shows a balance assertion as a chip, as the journal writes it", async () => {
        serve(asserted());
        await mountOverlay();
        const chips = screen.getAllByTestId("review-assertion");
        expect(chips.map((chip) => (chip.textContent ?? "").trim())).toEqual(["== $0.00"]);
        await fireEvent.click(screen.getByRole("button", {name: "Remove the balance assertion on posting 2"}));
        expect(screen.queryByTestId("review-assertion")).toBeNull();
    });

    it("shows an existing posting comment under its row, and ⋯ only on rows without one", async () => {
        serve(coffee({}, [leg("expenses:unknown", money(675n), {comment: "gift card"}), leg("liabilities:visa", money(-675n), {elided: true})]));
        await mountOverlay();
        expect(input("Comment for posting 1").value).toBe("gift card");
        expect(screen.queryByLabelText("Comment for posting 2")).toBeNull();
        await fireEvent.click(screen.getByRole("button", {name: "Add a comment to posting 2"}));
        await settle();
        expect(document.activeElement).toBe(input("Comment for posting 2"));
    });

    it("completes a commodity from the journal's", async () => {
        serve(coffee({}, [leg("expenses:unknown", money(675n, "EUR")), leg("liabilities:visa", money(-675n, "EUR"), {elided: true})]));
        await mountOverlay();
        const field = input("Commodity for posting 1");
        expect(field.value).toBe("EUR");
        field.focus();
        await fireEvent.input(field, {target: {value: ""}});
        await settle();
        const options = screen.getAllByRole("option").map((option) => (option.textContent ?? "").trim());
        expect(options).toEqual(["$", "EUR"]);
        await fireEvent.keyDown(field, {key: "Enter"});
        expect(field.value).toBe("$");
    });
});

describe("COMPONENT ReviewOverlay — the complete card over an import", () => {
    it("sends the details, a posting comment and the comment to the import's sink", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [])});
        const saves: StagedSave[] = [];
        const txn = coffee({comment: "first\nsecond"});
        reviewLaunch.openStaged({
            kind: "all",
            items: [decodeCsvStagedTxn(stagedTxnWire(txn, {ordinal: 0, key: "blue bottle"}), "staged")],
            sink: {stage: (staged) => (saves.push(staged), {ok: true})},
            rules: null,
            onClose: () => {},
        });
        render(ReviewOverlay, {props: {kind: "all", staged: reviewLaunch.staged}});
        await settle();
        expect(screen.getByTestId("review-details-summary")).toBeTruthy();
        card().focus();
        await press("!");
        await press("#");
        await type("Code", "9");
        await fireEvent.click(screen.getByRole("button", {name: "Add a comment to posting 1"}));
        await type("Comment for posting 1", "tip included");
        await type("Comment", "first\nsecond, edited");
        await fireEvent.input(accountField(), {target: {value: "expenses:food:coffee"}});
        await save();
        expect(saves[0]?.overrides[0]?.ops).toEqual([
            {op: "setStatus", status: "pending"},
            {op: "setCode", code: "9"},
            {op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"},
            {op: "setPostingComment", posting: 0, comment: "tip included"},
            {op: "setTxnComment", comment: "first\nsecond, edited"},
            REVIEWED,
        ]);
    });
});
