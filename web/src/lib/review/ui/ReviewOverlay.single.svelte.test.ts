// The card as the Journal's transaction editor (plan 32 Phase 7c, D31–D40):
// single mode's open and load, the Mark reviewed checkbox, add mode with its
// payee suggestions, delete, the keys, a conflict, and the behaviour the old
// TransactionModal's tests pinned (generated postings read-only, validation,
// Escape in a combobox, the modal key layer), now on the card.

import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {goto} from "$app/navigation";
import type {Amount, Posting, Transaction} from "#lib/domain/types.js";
import {txnModal} from "#lib/journal/edit/modalState.svelte.js";
import {keymap} from "#lib/keys/keymap.svelte.js";
import {editing} from "#lib/stores/editing.svelte.js";
import {localToday} from "#lib/stores/filters.svelte.js";
import {resetReview, review} from "#lib/stores/review.svelte.js";
import {autoSaleWire} from "#lib/testing/autoRow.js";
import {connectFakeEngine, FAKE_JOURNAL_INFO, journalInfoBody} from "#lib/testing/fakeEngine.js";
import {journal} from "#lib/stores/journal.svelte.js";
import KeysProbe from "#lib/testing/fixtures/KeysProbe.svelte";
import {deferred, queueItemWire, queuePageWire, reviewTxnWire, serveReview, similarWire, txnFixture, type SeenRequest} from "#lib/testing/reviewFixture.js";
import {accountField, card, MOD, mountOverlay, press, settle, suggestionWire} from "#lib/testing/reviewOverlay.js";
import {resetReviewLaunch, reviewLaunch} from "./launch.svelte";
import ReviewOverlay from "./ReviewOverlay.svelte";

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));

const TODAY = localToday();
const STYLE = {side: "L", spaced: false, precision: 2, decimalPoint: ".", digitGroups: null} as const;
const money = (cents: bigint): Amount => ({commodity: "$", qty: {m: cents, p: 2}, style: STYLE});
const leg = (account: string, amount: Amount | null, extra: Partial<Posting> = {}): Posting => ({
    account,
    amounts: amount === null ? [] : [amount],
    status: "unmarked",
    comment: "",
    tags: [],
    ...extra,
});

/** A categorised Safeway, #4, both legs written. */
function safeway(tags: [string, string][] = []): Transaction {
    return {
        index: 4,
        date: "2026-10-12",
        status: "unmarked",
        description: "Safeway",
        code: "",
        comment: "",
        tags,
        postings: [leg("expenses:food", money(500n)), leg("liabilities:visa", money(-500n))],
    };
}

type Edit = {index: number; fingerprint: string; ops: unknown[]};

interface Engine {
    /** `GET /api/review/item/{index}`: the card's item wire. */
    item?: Record<string, unknown>;
    /** Held until a test lets it go. */
    similar?: () => unknown;
    batch?: (edits: Edit[]) => unknown;
    add?: (body: unknown) => unknown;
    remove?: (index: number) => unknown;
    suggest?: (description: string) => unknown;
    payees?: string[];
    /** The journal's accounts, for the account fields' completion. */
    accounts?: string[];
}

/** Stub `fetch` as an engine serving single mode's routes, plus a journal refresh's. Every request is kept. */
function serveSingle(engine: Engine): SeenRequest[] {
    const seen: SeenRequest[] = [];
    const json = async (value: unknown, status = 200): Promise<Response> => {
        const resolved = await value;
        return resolved instanceof Response ? resolved : new Response(JSON.stringify(resolved), {status});
    };
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input));
        const method = init?.method ?? "GET";
        const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
        seen.push({method, url, body});
        const path = url.pathname;
        if (path.endsWith("/version")) return json("1.52");
        if (path.endsWith("/api/journal")) {
            const accounts = Object.fromEntries(
                (engine.accounts ?? []).map((name) => [name, {type: null, typeDeclared: false, term: null, termGuessed: false}])
            );
            return json(engine.accounts === undefined ? FAKE_JOURNAL_INFO : journalInfoBody({accounts, generation: `accounts-${engine.accounts.join(",")}`}));
        }
        if (path.endsWith("/api/diagnostics")) return json({count: 0, maxSeverity: null, diagnostics: []});
        if (path.endsWith("/api/payees")) {
            return json({generation: "g1", payees: (engine.payees ?? []).map((payee) => ({payee, count: 3, last: "2026-10-01"}))});
        }
        if (path.endsWith("/api/tags")) return json({generation: "g1", tags: []});
        if (/\/api\/review\/item\/\d+$/.test(path) && engine.item !== undefined) return json({generation: "g1", item: engine.item});
        const similar = /\/api\/review\/similar\/(\d+)$/.exec(path);
        if (similar !== null) return json(engine.similar?.() ?? similarWire(Number(similar[1]), []));
        if (path.endsWith("/api/review/suggest")) {
            const description = url.searchParams.get("description") ?? "";
            return json(engine.suggest?.(description) ?? {generation: "g1", key: null, suggestions: []});
        }
        if (path.endsWith("/api/transactions/batch") && engine.batch !== undefined) return json(engine.batch((body as {edits: Edit[]}).edits));
        if (path.endsWith("/api/transactions") && method === "POST" && engine.add !== undefined) return json(engine.add(body), 201);
        const one = /\/api\/transactions\/(\d+)$/.exec(path);
        if (one !== null && method === "DELETE" && engine.remove !== undefined) return json(engine.remove(Number(one[1])));
        return Promise.resolve(new Response(`no route for ${url}`, {status: 404}));
    });
    return seen;
}

/** Open the editor on #`index` (as Enter on a journal row does) and wait for its card. */
async function openEdit(index = 4): Promise<void> {
    txnModal.openEdit(index);
    render(ReviewOverlay, {props: {kind: "edit", index}});
    await vi.waitFor(() => {
        if (review.loading || review.current === null) throw new Error("card not loaded");
    });
    await settle();
}

/** Open a blank card (as `n` does). */
async function openAdd(): Promise<void> {
    txnModal.openAdd();
    render(ReviewOverlay, {props: {kind: "add"}});
    await settle();
}

const checkbox = (): HTMLInputElement => screen.getByTestId("review-mark-reviewed") as HTMLInputElement;
const save = (): HTMLButtonElement => screen.getByTestId("review-primary") as HTMLButtonElement;
const batches = (seen: SeenRequest[]): Edit[][] =>
    seen.filter((request) => request.url.pathname.endsWith("/api/transactions/batch")).map((request) => (request.body as {edits: Edit[]}).edits);
/** The ops of the one edit each save sent for the card. */
const opsSent = (seen: SeenRequest[]): unknown[][] => batches(seen).map((edits) => edits[0]?.ops ?? []);
const closed = async (): Promise<void> => vi.waitFor(() => expect(txnModal.open).toBe(false));
const ok = (txn: Transaction) => (edits: Edit[]) => ({generation: "g2", edited: edits.map(() => reviewTxnWire(txn, {unknown: false}))});
const helpText = (): string => JSON.stringify(keymap.help);

beforeEach(async () => {
    resetReview();
    resetReviewLaunch();
    txnModal.close();
    editing.clearConflict();
    await connectFakeEngine();
});
afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.mocked(goto).mockClear();
    keymap.reset();
});

describe("COMPONENT ReviewOverlay single mode: open and load", () => {
    it("opens on one transaction as 'Edit transaction', with no queue around it", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();

        expect(screen.getByRole("dialog", {name: "Edit transaction"})).toBeTruthy();
        expect(screen.queryByTestId("review-position")).toBeNull();
        expect(screen.queryByRole("progressbar")).toBeNull();
        expect(screen.queryByTestId("review-peek")).toBeNull();
        expect(screen.queryByTestId("review-skip")).toBeNull();
        expect(screen.getByTestId("review-footer").dataset.state).toBe("single");
        // Card mode, and the details row open (D33).
        expect(document.activeElement).toBe(card());
        expect(screen.getByTestId("review-details")).toBeTruthy();
        expect(screen.queryByRole("button", {name: "Hide the details"})).toBeNull();
    });

    it("is editable before the similar set lands", async () => {
        const held = deferred<unknown>();
        serveSingle({item: queueItemWire(safeway(), {unknown: false}), similar: () => held.promise});
        await openEdit();

        expect((screen.getByLabelText("Description") as HTMLInputElement).value).toBe("Safeway");
        expect(accountField(0).value).toBe("expenses:food");
        expect(accountField(0).disabled).toBe(false);
        held.resolve(similarWire(4, [reviewTxnWire(txnFixture(9, "Safeway", "expenses:food"))]));
        await vi.waitFor(() => expect(screen.queryByTestId("review-similar")).not.toBeNull());
    });

    it("says why when the transaction can't be loaded", async () => {
        serveSingle({});
        await openEditUnloaded();
        expect(screen.getByRole("alert").textContent).toContain("Couldn't load the transaction");
    });
});

/** Open on a transaction the engine doesn't have, and wait for the failure. */
async function openEditUnloaded(): Promise<void> {
    txnModal.openEdit(99);
    render(ReviewOverlay, {props: {kind: "edit", index: 99}});
    await vi.waitFor(() => expect(review.error).not.toBeNull());
    await settle();
}

describe("COMPONENT ReviewOverlay single mode: Mark reviewed (D32)", () => {
    it.each([
        ["unreviewed", [] as [string, string][], false, "Mark reviewed"],
        ["reviewed", [["reviewed", "2026-10-30"]] as [string, string][], true, "Mark reviewed"],
        ["ignored", [["reviewed", "ignore"]] as [string, string][], true, "Ignored"],
    ])("starts as the transaction is: %s", async (_name, tags, isChecked, label) => {
        serveSingle({item: queueItemWire(safeway(tags), {unknown: false, reviewed: tags.length > 0})});
        await openEdit();

        expect(checkbox().checked).toBe(isChecked);
        expect(checkbox().disabled).toBe(false);
        expect(checkbox().closest("label")?.textContent).toContain(label);
        expect(save().disabled).toBe(true);
    });

    it("is disabled while a posting is unknown, with the queue card's reason", async () => {
        serveSingle({item: queueItemWire(txnFixture(4, "BLUE BOTTLE"))});
        await openEdit();

        expect(checkbox().disabled).toBe(true);
        expect(checkbox().closest("[data-tip]")?.getAttribute("data-tip")).toContain("Categorise the unknown posting first");
    });

    it("checking stamps today, and counts as a change on its own", async () => {
        const seen = serveSingle({item: queueItemWire(safeway(), {unknown: false}), batch: ok(safeway())});
        await openEdit();

        await fireEvent.click(checkbox());
        expect(checkbox().checked).toBe(true);
        expect(save().disabled).toBe(false);
        await fireEvent.click(save());
        await closed();

        expect(opsSent(seen)).toEqual([[{op: "setTxnTag", name: "reviewed", value: TODAY}]]);
    });

    it("unchecking removes the tag", async () => {
        const seen = serveSingle({item: queueItemWire(safeway([["reviewed", "2026-10-30"]]), {unknown: false, reviewed: true}), batch: ok(safeway())});
        await openEdit();

        await fireEvent.click(checkbox());
        await press("Enter", MOD);
        await closed();

        expect(opsSent(seen)).toEqual([[{op: "setTxnTag", name: "reviewed", value: null}]]);
    });

    it("left checked, a reviewed transaction keeps its date", async () => {
        const seen = serveSingle({item: queueItemWire(safeway([["reviewed", "2026-10-30"]]), {unknown: false, reviewed: true}), batch: ok(safeway())});
        await openEdit();

        await fireEvent.input(screen.getByLabelText("Comment"), {target: {value: "split with Sam"}});
        card().focus();
        await press("Enter", MOD);
        await closed();

        expect(opsSent(seen)).toEqual([[{op: "setTxnComment", comment: "split with Sam"}]]);
    });

    it("unchecked and checked again, it keeps the original date", async () => {
        serveSingle({item: queueItemWire(safeway([["reviewed", "2026-10-30"]]), {unknown: false, reviewed: true})});
        await openEdit();

        await press("v");
        expect(checkbox().checked).toBe(false);
        await press("v");
        expect(checkbox().checked).toBe(true);
        expect(save().disabled).toBe(true);
    });

    it("v toggles it from card mode", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();

        await press("v");
        expect(checkbox().checked).toBe(true);
        expect(save().disabled).toBe(false);
        await press("v");
        expect(checkbox().checked).toBe(false);
    });

    it("⌘⇧↵ accepts the chip and saves WITHOUT marking reviewed when the box is unchecked", async () => {
        const txn = txnFixture(4, "BLUE BOTTLE");
        const seen = serveSingle({item: queueItemWire(txn, {suggestions: [suggestionWire("expenses:food:coffee")]}), batch: ok(txn)});
        await openEdit();

        await press("Enter", {...MOD, shiftKey: true});
        await closed();

        expect(opsSent(seen)).toEqual([[{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}]]);
    });

    it("⌘⇧↵ marks it reviewed when the box is checked", async () => {
        const txn = txnFixture(4, "BLUE BOTTLE");
        const seen = serveSingle({item: queueItemWire(txn, {suggestions: [suggestionWire("expenses:food:coffee")]}), batch: ok(txn)});
        await openEdit();

        // The chip first: until the unknown leg has an account, the box can't be checked.
        await fireEvent.click(screen.getByTestId("review-chip"));
        card().focus();
        await press("v");
        expect(checkbox().checked).toBe(true);
        await press("Enter", {...MOD, shiftKey: true});
        await closed();

        expect(opsSent(seen)).toEqual([
            [
                {op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"},
                {op: "setTxnTag", name: "reviewed", value: TODAY},
            ],
        ]);
    });

    it("never stamps the similar transactions it applies to", async () => {
        const txn = txnFixture(4, "BLUE BOTTLE");
        const sibling = txnFixture(9, "BLUE BOTTLE");
        const seen = serveSingle({
            item: queueItemWire(txn, {suggestions: [suggestionWire("expenses:food:coffee")]}),
            similar: () => similarWire(4, [reviewTxnWire(sibling)]),
            batch: ok(txn),
        });
        await openEdit();

        await fireEvent.click(screen.getByTestId("review-chip"));
        card().focus();
        await press("v");
        await press("s");
        await fireEvent.click(within(screen.getByTestId("review-similar")).getAllByRole("checkbox").at(-1) as HTMLElement);
        await fireEvent.click(save());
        await closed();

        const [edits] = batches(seen);
        expect(edits?.map((edit) => edit.index)).toEqual([4, 9]);
        expect(edits?.[1]?.ops).toEqual([{op: "setPostingAccount", posting: 0, account: "expenses:food:coffee"}]);
    });
});

describe("COMPONENT ReviewOverlay single mode: saving", () => {
    it("closes after the save and puts the journal's cursor on the transaction", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false}), batch: ok(safeway())});
        await openEdit();

        await press("v");
        await press("Enter", MOD);
        await vi.waitFor(() => expect(screen.queryByTestId("review-flash")).not.toBeNull());
        await closed();

        expect(reviewLaunch.returnTo?.txnIndex).toBe(4);
    });

    it("REGRESSION: a 409 closes the editor and raises the page's changed-on-disk banner", async () => {
        serveSingle({
            item: queueItemWire(safeway(), {unknown: false}),
            batch: () => new Response("transaction #4 changed on disk since it was read", {status: 409}),
        });
        await openEdit();

        await press("v");
        await press("Enter", MOD);
        await closed();

        expect(editing.conflict).toBe(true);
        expect(reviewLaunch.returnTo).toBeNull();
    });

    it("shows a refusal on the card and keeps the edits", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false}), batch: () => new Response("transaction #4: the postings don't balance", {status: 400})});
        await openEdit();

        await press("v");
        await press("Enter", MOD);
        await vi.waitFor(() => expect(screen.queryByTestId("review-failure")).not.toBeNull());

        expect(screen.getByTestId("review-failure").textContent).toContain("don't balance");
        expect(txnModal.open).toBe(true);
        expect(checkbox().checked).toBe(true);
    });

    it("refuses an amount it can't read, and sends nothing", async () => {
        const seen = serveSingle({item: queueItemWire(safeway(), {unknown: false}), batch: ok(safeway())});
        await openEdit();

        await fireEvent.input(screen.getByLabelText("Amount for posting 1"), {target: {value: "five"}});
        card().focus();
        await press("Enter", MOD);

        expect(screen.getByTestId("review-errors").textContent).toContain('"five" is not a valid amount.');
        expect(batches(seen)).toEqual([]);
        // The message follows the edit.
        await fireEvent.input(screen.getByLabelText("Amount for posting 1"), {target: {value: "5.25"}});
        expect(screen.queryByTestId("review-errors")).toBeNull();
    });
});

describe("COMPONENT ReviewOverlay single mode: add (D36)", () => {
    const netflix = {
        generation: "g1",
        key: "netflix",
        suggestions: [
            suggestionWire("expenses:streaming", true, {
                source: "recurring",
                recurring: {amount: {commodity: "$", quantity: {mantissa: "1549", places: 2}}, account: "liabilities:visa"},
            }),
        ],
    };

    it("opens blank: today, unmarked, two postings, focus in the description, unchecked", async () => {
        serveSingle({});
        await openAdd();

        expect(screen.getByRole("dialog", {name: "Add transaction"})).toBeTruthy();
        expect(document.activeElement).toBe(screen.getByLabelText("Description"));
        expect((screen.getByLabelText("Date") as HTMLInputElement).value).toBe(TODAY);
        expect(screen.getByLabelText(/^Status: unmarked/)).toBeTruthy();
        expect(screen.getAllByTestId("review-posting")).toHaveLength(2);
        expect(checkbox().checked).toBe(false);
        expect(save().textContent).toContain("Add");
        // Nothing to apply to, nothing to remember: it is no transaction yet.
        expect(screen.queryByTestId("review-similar")).toBeNull();
        expect(screen.queryByTestId("review-rule")).toBeNull();
        expect(screen.queryByTestId("review-split-note")).toBeNull();
    });

    it("picking a payee offers its usual category, with the recurring amount and account", async () => {
        const seen = serveSingle({payees: ["Netflix"], suggest: () => netflix});
        await openAdd();

        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Net"}});
        await fireEvent.click(screen.getByRole("option", {name: /Netflix/}));
        await settle();

        const asked = seen.find((request) => request.url.pathname.endsWith("/api/review/suggest"));
        expect(asked?.url.searchParams.get("description")).toBe("Netflix");
        const chip = screen.getByTestId("review-chip");
        expect(chip.textContent).toContain("streaming");
        expect(within(chip).getByTestId("review-chip-fill").textContent).toBe("$15.49 from liabilities:visa");
        expect(chip.getAttribute("aria-pressed")).toBe("true");
    });

    it("accepting the recurring chip fills the amount and the funding account", async () => {
        serveSingle({payees: ["Netflix"], suggest: () => netflix});
        await openAdd();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Net"}});
        await fireEvent.click(screen.getByRole("option", {name: /Netflix/}));
        await settle();

        await fireEvent.click(screen.getByTestId("review-chip"));

        expect(accountField(0).value).toBe("expenses:streaming");
        expect((screen.getByLabelText("Amount for posting 1") as HTMLInputElement).value).toBe("15.49");
        expect(accountField(1).value).toBe("liabilities:visa");
        expect((screen.getByLabelText("Amount for posting 2") as HTMLInputElement).value).toBe("");
    });

    it("⌘⇧↵ accepts the chip and adds the transaction in one POST, then lands the cursor on it", async () => {
        const seen = serveSingle({payees: ["Netflix"], suggest: () => netflix, add: () => ({index: 42, transaction: {}})});
        await openAdd();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Net"}});
        await fireEvent.click(screen.getByRole("option", {name: /Netflix/}));
        await settle();

        await press("Enter", {...MOD, shiftKey: true});
        await closed();

        const posts = seen.filter((request) => request.method === "POST" && request.url.pathname.endsWith("/api/transactions"));
        expect(posts.map((request) => request.body)).toEqual([
            {
                date: TODAY,
                description: "Netflix",
                postings: [{account: "expenses:streaming", amount: {commodity: "$", quantity: {mantissa: "1549", places: 2}}}, {account: "liabilities:visa"}],
            },
        ]);
        expect(reviewLaunch.returnTo?.txnIndex).toBe(42);
    });

    it("checked, the stamp rides the comment of the one POST", async () => {
        const seen = serveSingle({add: () => ({index: 42, transaction: {}})});
        await openAdd();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Corner store"}});
        await fireEvent.input(accountField(0), {target: {value: "expenses:food"}});
        await fireEvent.input(screen.getByLabelText("Amount for posting 1"), {target: {value: "4"}});
        await fireEvent.input(accountField(1), {target: {value: "assets:cash"}});
        await fireEvent.input(screen.getByLabelText("Comment"), {target: {value: "milk"}});
        card().focus();
        await press("v");
        await press("Enter", MOD);
        await closed();

        const post = seen.find((request) => request.method === "POST" && request.url.pathname.endsWith("/api/transactions"));
        expect(post?.body).toMatchObject({description: "Corner store", comment: `milk, reviewed:${TODAY}`});
    });

    it("an empty card can't be added", async () => {
        serveSingle({});
        await openAdd();
        expect(save().disabled).toBe(true);
    });
});

describe("COMPONENT ReviewOverlay single mode: delete (D37)", () => {
    it("⇧X arms the delete in the footer, and ⇧X again deletes and closes", async () => {
        const seen = serveSingle({item: queueItemWire(safeway(), {unknown: false}), remove: (index) => ({deletedIndex: index, remaining: 3})});
        await openEdit();

        await press("X", {shiftKey: true});
        const bar = screen.getByTestId("review-confirm-delete");
        expect(within(bar).getByRole("button", {name: /Confirm delete/})).toBeTruthy();
        expect(within(bar).getByRole("button", {name: "Keep"})).toBeTruthy();
        expect(seen.some((request) => request.method === "DELETE")).toBe(false);

        await press("X", {shiftKey: true});
        await closed();

        expect(seen.filter((request) => request.method === "DELETE").map((request) => request.url.pathname)).toEqual(["/api/transactions/4"]);
        // The table's delete leaves the cursor where it was: no jump.
        expect(reviewLaunch.returnTo).toBeNull();
    });

    it("Keep, or Escape, disarms it", async () => {
        const seen = serveSingle({item: queueItemWire(safeway(), {unknown: false}), remove: (index) => ({deletedIndex: index, remaining: 3})});
        await openEdit();

        await fireEvent.click(screen.getByTestId("review-delete"));
        await fireEvent.click(screen.getByRole("button", {name: "Keep"}));
        expect(screen.queryByTestId("review-confirm-delete")).toBeNull();
        await press("X", {shiftKey: true});
        await press("Escape");
        expect(screen.queryByTestId("review-confirm-delete")).toBeNull();

        expect(txnModal.open).toBe(true);
        expect(seen.some((request) => request.method === "DELETE")).toBe(false);
    });

    it("is not offered on an add card", async () => {
        serveSingle({});
        await openAdd();
        expect(screen.queryByTestId("review-delete")).toBeNull();
        expect(helpText()).not.toContain("Delete the transaction");
    });
});

describe("COMPONENT ReviewOverlay single mode: keys (D40)", () => {
    it("adds v and ⇧X, and drops Skip, Back, Ignore and Reviewed-and-next", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();

        const help = helpText();
        for (const label of [
            "Mark reviewed, or not",
            "Delete the transaction",
            "Edit the date",
            "Cycle the status",
            "Edit the code",
            "Accept the suggestion and save",
        ]) {
            expect(help).toContain(label);
        }
        for (const label of ["Skip", "Previous card", "Ignore (never ask again)", "mark reviewed, next", "Open in the full editor"]) {
            expect(help).not.toContain(label);
        }
    });

    it("n, → and ← do nothing to the card", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();

        for (const key of ["n", "ArrowRight", "ArrowLeft", "p"]) await press(key);

        expect(screen.getByTestId("review-card")).toBeTruthy();
        expect(document.activeElement).toBe(card());
    });

    it("keeps the field keys: d, 1, $, t, m, # and D", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();

        const cases: [string, () => Element][] = [
            ["d", () => screen.getByLabelText("Description")],
            ["1", () => accountField(0)],
            ["$", () => screen.getByLabelText("Amount for posting 1")],
            ["t", () => screen.getByLabelText("Add a tag")],
            ["m", () => screen.getByLabelText("Comment")],
            ["#", () => screen.getByLabelText("Code")],
            ["D", () => screen.getByLabelText("Date")],
        ];
        for (const [key, field] of cases) {
            card().focus();
            await press(key);
            expect(document.activeElement, key).toBe(field());
        }
    });

    it("Escape goes field → card → close, asking first when the card is dirty", async () => {
        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();

        await press("m");
        await fireEvent.input(screen.getByLabelText("Comment"), {target: {value: "split with Sam"}});
        await press("Escape");
        expect(document.activeElement).toBe(card());
        await press("Escape");
        expect(screen.getByTestId("review-confirm-close")).toBeTruthy();
        await fireEvent.click(screen.getByRole("button", {name: "Discard and close"}));
        await closed();
    });

    it("REGRESSION: Escape in an account field with suggestions open closes the suggestions, not the editor", async () => {
        serveSingle({accounts: ["expenses:groceries:costco", "expenses:gas"]});
        await journal.refresh({force: true});
        await openAdd();
        await fireEvent.input(screen.getByLabelText("Description"), {target: {value: "Plumber"}});
        const account = accountField(0);
        account.focus();
        await fireEvent.input(account, {target: {value: "ex"}});
        await settle();
        expect(screen.queryByRole("listbox")).not.toBeNull();

        await press("Escape");

        expect(screen.queryByRole("listbox")).toBeNull();
        expect(txnModal.open).toBe(true);
        expect((screen.getByLabelText("Description") as HTMLInputElement).value).toBe("Plumber");
        // The next Escape leaves the field; the one after asks (the card has edits).
        await press("Escape");
        expect(document.activeElement).toBe(card());
        await press("Escape");
        expect(screen.getByTestId("review-confirm-close")).toBeTruthy();
    });

    it("blinds the journal's keys while open, and only then", async () => {
        const next = vi.fn();
        render(KeysProbe, {
            props: {layerOf: () => ({id: "journal", priority: 10, bindings: [{keys: "j", label: "Next", group: "Journal" as const, run: next}]})},
        });
        const j = () => keymap.handle(new KeyboardEvent("keydown", {key: "j", cancelable: true}));
        j();
        expect(next).toHaveBeenCalledTimes(1);

        serveSingle({item: queueItemWire(safeway(), {unknown: false})});
        await openEdit();
        j();
        expect(next).toHaveBeenCalledTimes(1);
    });
});

describe("COMPONENT ReviewOverlay single mode: postings", () => {
    it("edits only the written postings and lists the rule's below, read-only", async () => {
        serveSingle({item: {...autoSaleWire(), unknown: false, reviewed: false, fingerprint: "fp-4-0", key: "acme", suggestions: []}});
        await openEdit();

        expect([accountField(0).value, accountField(1).value]).toEqual(["revenue:sales", "assets:checking"]);
        expect(screen.queryByLabelText("Account for posting 3")).toBeNull();
        const generated = screen.getAllByTestId("review-generated");
        expect(generated.map((row) => row.textContent)).toEqual([expect.stringContaining("liabilities:tax:sales"), expect.stringContaining("assets:checking")]);
        expect(generated[0]?.textContent).toContain("= revenue:sales");
        expect(generated[0]?.querySelector("input")).toBeNull();
    });

    it("closes and opens the rule when its link is clicked", async () => {
        serveSingle({item: {...autoSaleWire(), unknown: false, reviewed: false, fingerprint: "fp-4-0", key: "acme", suggestions: []}});
        await openEdit();

        await fireEvent.click(screen.getAllByRole("button", {name: /from automatic posting rule/})[0] as HTMLElement);

        await closed();
        expect(goto).toHaveBeenCalledWith("/settings?tab=auto&rule=0");
    });

    it("an elided leg's commodity waits for an amount", async () => {
        const txn = {...safeway(), postings: [leg("expenses:food", money(500n)), leg("liabilities:visa", money(-500n), {elided: true})]};
        serveSingle({item: queueItemWire(txn, {unknown: false})});
        await openEdit();

        const commodity = screen.getByLabelText("Commodity for posting 2") as HTMLInputElement;
        expect(commodity.disabled).toBe(true);
        expect(commodity.title).toBe("Type an amount to set its commodity");
        await fireEvent.input(screen.getByLabelText("Amount for posting 2"), {target: {value: "-5"}});
        expect(commodity.disabled).toBe(false);
        expect((screen.getByLabelText("Commodity for posting 1") as HTMLInputElement).disabled).toBe(false);
    });
});

describe("COMPONENT ReviewOverlay queue card", () => {
    it("has no 'Open in full editor': the card is the full editor now (D39)", async () => {
        serveReview({queue: () => queuePageWire("unknowns", [queueItemWire(txnFixture(1, "BLUE BOTTLE"))])});
        await mountOverlay("unknowns");

        expect(screen.queryByRole("button", {name: /full editor/})).toBeNull();
        expect(helpText()).not.toContain("full editor");
        expect(screen.queryByTestId("review-mark-reviewed")).toBeNull();
        expect(screen.queryByTestId("review-delete")).toBeNull();
    });
});
