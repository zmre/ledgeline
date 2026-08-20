import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import type {Dec} from "$lib/domain/money";
import type {Amount, AmountStyle, ISODate, Posting, Transaction} from "$lib/domain/types";
import {contentFingerprint, journal} from "./journal.svelte";
import {settings} from "./settings.svelte";

function posting(account: string): Posting {
    return {account, amounts: [], status: "unmarked", comment: "", tags: []};
}

const STYLE: AmountStyle = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};

function amount(commodity: string, m: bigint, p: number): Amount {
    return {commodity, qty: {m, p} as Dec, style: STYLE};
}

/** A txn whose single posting carries `amounts`, with a caller-supplied haystack. */
function txnWithAmounts(haystack: string, amounts: Amount[]): Transaction {
    return {
        index: 1,
        date: "2026-06-29",
        status: "cleared",
        description: "Dividend",
        code: "",
        comment: "",
        tags: [],
        postings: [{account: "expenses:fees:broker", amounts, status: "unmarked", comment: "", tags: []}],
        haystack,
    };
}

function txn(index: number, date: ISODate, haystack: string): Transaction {
    return {index, date, status: "cleared", description: "t", code: "", comment: "", tags: [], postings: [posting("expenses:food")], haystack};
}

describe("UNIT journal contentFingerprint", () => {
    const base = [txn(1, "2026-01-05", "groceries $100.00 expenses:food"), txn(2, "2026-07-05", "rent $1,800.00 expenses:housing")];

    it("is stable for identical content", () => {
        const same = [txn(1, "2026-01-05", "groceries $100.00 expenses:food"), txn(2, "2026-07-05", "rent $1,800.00 expenses:housing")];
        expect(contentFingerprint(same, ["expenses"], [])).toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when a MID-LIST transaction is edited in place (count and last txn unchanged)", () => {
        const edited = [txn(1, "2026-01-05", "groceries $999.00 expenses:food"), txn(2, "2026-07-05", "rent $1,800.00 expenses:housing")];
        expect(contentFingerprint(edited, ["expenses"], [])).not.toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when an amount moves BELOW the 2-place display cap", () => {
        // A real case: clearing a $-0.16 broker fee on a dividend reinvest lets
        // the engine infer $-0.16343. The haystack is built for SEARCH, so its
        // amounts are rounded to 2 places and both render "$-0.16" — identical
        // text. If the fingerprint trusted only that, the post-edit refresh
        // would fetch the corrected transaction and then throw it away, leaving
        // the old amount on screen and the "unbalanced" problem in the badge.
        const haystack = "dividend\nexpenses:fees:broker\n$-0.16\n$";
        const rounded = txnWithAmounts(haystack, [amount("$", -16n, 2)]);
        const exact = txnWithAmounts(haystack, [amount("$", -16343n, 5)]);
        expect(rounded.haystack).toBe(exact.haystack); // the trap this guards
        expect(contentFingerprint([exact], [], [])).not.toBe(contentFingerprint([rounded], [], []));
    });

    it("changes when a share count differs past the 2-place display cap", () => {
        // Same trap for quantities: 15.244 and 15.245 shares both display "15.24".
        const haystack = "reinvest\nassets:broker\n15.24 VWEHX\nvwehx";
        const a = txnWithAmounts(haystack, [amount("VWEHX", 15244n, 3)]);
        const b = txnWithAmounts(haystack, [amount("VWEHX", 15245n, 3)]);
        expect(contentFingerprint([a], [], [])).not.toBe(contentFingerprint([b], [], []));
    });

    it("changes when only a cost annotation changes", () => {
        const haystack = "reinvest\nassets:broker\n0.29 VFIAX\nvfiax";
        const withCost = (costM: bigint): Transaction =>
            txnWithAmounts(haystack, [{...amount("VFIAX", 293n, 3), cost: {commodity: "$", qty: {m: costM, p: 2} as Dec, per: true}}]);
        expect(contentFingerprint([withCost(67851n)], [], [])).not.toBe(contentFingerprint([withCost(67852n)], [], []));
    });

    it("is still stable when nothing changed, amounts included", () => {
        const make = (): Transaction => txnWithAmounts("dividend\nexpenses:fees:broker\n$-0.16\n$", [amount("$", -16343n, 5)]);
        expect(contentFingerprint([make()], [], [])).toBe(contentFingerprint([make()], [], []));
    });

    it("changes when a txn date or status changes", () => {
        const redated = [txn(1, "2026-01-06", "groceries $100.00 expenses:food"), base[1]];
        expect(contentFingerprint(redated, ["expenses"], [])).not.toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when account names or prices change", () => {
        expect(contentFingerprint(base, ["expenses", "assets"], [])).not.toBe(contentFingerprint(base, ["expenses"], []));
        const style = {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};
        const price = {date: "2026-07-01", commodity: "EUR", price: {commodity: "$", qty: {m: 117n, p: 2}, style}};
        expect(contentFingerprint(base, ["expenses"], [price])).not.toBe(contentFingerprint(base, ["expenses"], []));
    });

    it("changes when a price's SCALE changes but its mantissa does not", () => {
        // FE-4. The old line mixed `Number(BigInt.asIntN(32, qty.m))` — the
        // mantissa alone, truncated — so `P VTI $1.00` (m=100n, p=2) and
        // `P VTI $100` (m=100n, p=0) hashed identically. A price correction of
        // exactly that shape was fetched and then discarded, and because
        // doRefresh rewrites lastFingerprint even when it skips the swap, it was
        // discarded permanently: a stale portfolio on screen indefinitely.
        const style = {side: "L" as const, spaced: false, precision: 2, decimalPoint: ".", digitGroups: null};
        const priced = (m: bigint, p: number) => [{date: "2026-07-01", commodity: "VTI", price: {commodity: "$", qty: {m, p} as Dec, style}}];
        expect(contentFingerprint(base, [], priced(100n, 2))).not.toBe(contentFingerprint(base, [], priced(100n, 0)));
    });

    it("changes when two price mantissas differ by exactly 2^32 (the truncation collision)", () => {
        const style = {side: "L" as const, spaced: false, precision: 8, decimalPoint: ".", digitGroups: null};
        const priced = (m: bigint) => [{date: "2026-07-01", commodity: "BTC", price: {commodity: "$", qty: {m, p: 8} as Dec, style}}];
        // Both truncate to the same int32 under BigInt.asIntN(32, m).
        expect(contentFingerprint(base, [], priced(100_000_000n))).not.toBe(contentFingerprint(base, [], priced(100_000_000n + 4_294_967_296n)));
    });

    it("changes when a single POSTING's status changes (marking one leg cleared)", () => {
        // Posting status is absent from the haystack, so this edit hashed the
        // same and was fetched-then-discarded by the same mechanism.
        const withStatus = (s: Posting["status"]): Transaction => ({
            ...txn(1, "2026-01-05", "groceries"),
            postings: [{...posting("expenses:food"), status: s}],
        });
        expect(contentFingerprint([withStatus("cleared")], [], [])).not.toBe(contentFingerprint([withStatus("unmarked")], [], []));
    });

    it("changes when a posting date, a posting type or a balance assertion changes", () => {
        const t = txn(1, "2026-01-05", "groceries");
        const withPosting = (p: Posting): Transaction => ({...t, postings: [p]});
        const plain = posting("assets:cash");
        expect(contentFingerprint([withPosting({...plain, date: "2026-01-07"})], [], [])).not.toBe(contentFingerprint([withPosting(plain)], [], []));
        expect(contentFingerprint([withPosting({...plain, type: "virtual"})], [], [])).not.toBe(contentFingerprint([withPosting(plain)], [], []));
        const assertion = {amount: amount("$", 500n, 2), inclusive: false, total: false};
        expect(contentFingerprint([withPosting({...plain, balanceAssertion: assertion})], [], [])).not.toBe(contentFingerprint([withPosting(plain)], [], []));
    });

    it("changes when a transaction's SECONDARY date changes", () => {
        const t = txn(1, "2026-01-05", "groceries");
        expect(contentFingerprint([{...t, date2: "2026-01-09"}], [], [])).not.toBe(contentFingerprint([t], [], []));
    });

    it("changes when a declared account type changes (cash-flow must recompute)", () => {
        const asAsset = [{name: "assets:bank:checking", type: "asset" as const}];
        const asCash = [{name: "assets:bank:checking", type: "cash" as const}];
        expect(contentFingerprint(base, ["expenses"], [], asCash)).not.toBe(contentFingerprint(base, ["expenses"], [], asAsset));
        // The default (no decls) stays backward-compatible with the 3-arg call.
        expect(contentFingerprint(base, ["expenses"], [])).toBe(contentFingerprint(base, ["expenses"], [], []));
    });
});

// ---------------------------------------------------------------------------
// The journal's TITLE — what the app bar labels the screen with.
//
// Driven through a stubbed `fetch` like journalRefresh.test.ts, because every
// property here is about WHEN the label is written relative to the round that
// fetched it: after the supersession guard (so a late older round cannot
// relabel the screen), before the 304 early return (so an unchanged journal
// still refreshes the name), and outside the fingerprint that gates the
// transaction swap.
// ---------------------------------------------------------------------------

const json = (body: unknown): Response => new Response(JSON.stringify(body), {status: 200, headers: {"Content-Type": "application/json"}});

/** One wire transaction, enough for normalizeTransactions. */
const wireTxn = (index: number): unknown => ({
    tindex: index,
    tdate: `2026-01-0${index}`,
    tstatus: "Cleared",
    tdescription: `txn ${index}`,
    tpostings: [{paccount: "expenses:food", pamount: []}],
});

/** A promise with its settle function exposed, for gating a round mid-flight. */
function deferred<T>(): {promise: Promise<T>; resolve: (value: T) => void} {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((res) => {
        resolve = res;
    });
    return {promise, resolve};
}

describe("INTEGRATION journal title (which ledger is on screen)", () => {
    /**
     * What `/api/journal` answers, one entry per ROUND (the last repeats).
     *
     * A string is a title; `null` is a 404 — the answer a plain hledger-web
     * gives, since `/api/*` is native and it has none of those routes.
     */
    let titles: (string | null)[] = ["Acme Books"];
    let journalHits = 0;
    /** Answers `/transactions`; replaced by tests that need to gate or supersede a round. */
    let txnAnswer: (attempt: number) => Promise<Response> = () => Promise.resolve(json([wireTxn(1)]));
    let txnHits = 0;
    /** When true, the three conditional journal routes answer 304 (an unchanged journal). */
    let unchanged = false;

    beforeEach(async () => {
        titles = ["Acme Books"];
        journalHits = 0;
        txnHits = 0;
        txnAnswer = () => Promise.resolve(json([wireTxn(1)]));
        unchanged = false;
        vi.stubGlobal("fetch", (input: RequestInfo | URL) => {
            const url = String(input);
            if (url.endsWith("/api/journal")) {
                const title = titles[Math.min(journalHits, titles.length - 1)];
                journalHits += 1;
                // A 404 is what a server WITHOUT the route says; the client maps
                // it to NativeApiUnavailableError.
                return Promise.resolve(title === null ? new Response("no such route", {status: 404}) : json({title, file: "2026.journal"}));
            }
            if (url.endsWith("/api/diagnostics")) return Promise.resolve(json({diagnostics: []}));
            if (url.endsWith("/version")) return Promise.resolve(json("1.52"));
            if (url.endsWith("/accountnames")) return Promise.resolve(json(["expenses:food"]));
            if (unchanged && (url.endsWith("/transactions") || url.endsWith("/prices") || url.endsWith("/accounts"))) {
                return Promise.resolve(new Response(null, {status: 304}));
            }
            if (url.endsWith("/transactions")) {
                const attempt = txnHits;
                txnHits += 1;
                return txnAnswer(attempt);
            }
            return Promise.resolve(json([]));
        });
        await settings.setServerUrl("http://engine-a");
        // Leave the store loaded and idle, so each test starts from the same
        // place regardless of what ran before it.
        await journal.refresh({force: true});
    });

    afterEach(async () => {
        titles = ["Acme Books"];
        unchanged = false;
        txnAnswer = () => Promise.resolve(json([wireTxn(1)]));
        // Drain: abort whatever a test left running so it cannot bleed into the next.
        await journal.refresh({force: true}).catch(() => undefined);
        vi.unstubAllGlobals();
    });

    it("exposes the engine's title and journal file after a successful round", () => {
        expect(journal.title).toBe("Acme Books");
        expect(journal.file).toBe("2026.journal");
    });

    it("relabels when the SAME url starts serving a different journal (desktop File→Open)", () => {
        // The reason this rides the journal round at all. File→Open rebinds the
        // engine to another journal without changing the address, the nonce or
        // anything else a once-per-connection fetch would notice — so a title
        // fetched once would go on naming the previous entity all session.
        titles = ["Ledger Two"];
        journalHits = 0;
        return journal.refresh({force: true}).then(() => {
            expect(journal.title).toBe("Ledger Two");
        });
    });

    it("still loads the journal when the engine has no /api/journal, and names no ledger", async () => {
        // A plain hledger-web. The route 404s, the app bar shows no label at
        // all, and — the point — the journal itself loads exactly as before.
        titles = [null];
        journalHits = 0;
        await journal.refresh({force: true});

        expect(journal.status).toBe("ready");
        expect(journal.error).toBeNull();
        expect(journal.txns).toHaveLength(1);
        expect(journal.title).toBeNull();
        expect(journal.file).toBeNull();
    });

    it("clears a title it can no longer confirm rather than keeping a stale name", async () => {
        expect(journal.title).toBe("Acme Books");
        // Reconnected to something that cannot say which ledger it holds. The
        // honest label is the URL, not the last entity we happened to know about.
        titles = [null];
        journalHits = 0;
        await journal.refresh({force: true});
        expect(journal.title).toBeNull();
    });

    it("refreshes the label on a 304 round, where nothing else is swapped", async () => {
        // `/api/journal` is not conditional, so an unchanged journal still
        // answers it. The assignment therefore sits BEFORE the 304 early return:
        // put it after and a renamed-but-otherwise-unchanged journal would keep
        // the old name until something in the file happened to change.
        titles = ["Renamed Books"];
        journalHits = 0;
        unchanged = true;
        await journal.refresh({force: true});

        expect(journal.title).toBe("Renamed Books");
        expect(journal.status).toBe("ready");
    });

    it("does not let a superseded round relabel the screen", async () => {
        // The token guard, for the title. A round that answers after being
        // superseded describes a journal the app has already moved on from;
        // writing its name would leave the newest data under the previous
        // ledger's label — the exact confusion this feature exists to prevent.
        const gate = deferred<Response>();
        titles = ["Old Books", "New Books"];
        journalHits = 0;
        txnHits = 0;
        // Deliberately ignores the abort signal: a fetch that had already
        // completed when the abort landed behaves exactly like this.
        txnAnswer = (attempt) => (attempt === 0 ? gate.promise : Promise.resolve(json([wireTxn(1), wireTxn(2)])));

        const superseded = journal.refresh({force: true});
        const winner = journal.refresh({force: true});
        await winner;
        expect(journal.title).toBe("New Books");

        gate.resolve(json([wireTxn(1)]));
        await superseded;
        expect(journal.title).toBe("New Books");
    });
});
