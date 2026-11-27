// The Entries and Accounts calls: each sends what `app/time.rs` reads, the
// answers are checked for their shape, and a refusal keeps its code and the
// server's rewrite (`note-tag`'s suggestion).
import {afterEach, describe, expect, it, vi} from "vitest";
import {accountsView, entry, list} from "../testing.js";
import {archiveAccount, declareAccount, fetchAccounts, isAccountsView, renameAccount, setAccountTag} from "./accounts.js";
import {addEntry, bulkEntries, deleteEntry, editEntry, fetchEntries, isEntryList, refusal} from "./entries.js";

function answer(status: number, body: unknown) {
    const fetch = vi.fn<(url: string, init?: RequestInit) => Promise<Response>>(
        async () => new Response(JSON.stringify(body), {status, headers: {"Content-Type": "application/json"}})
    );
    vi.stubGlobal("fetch", fetch);
    return fetch;
}

const call = (fetch: ReturnType<typeof answer>, at = 0) => {
    const [url, init] = fetch.mock.calls[at];
    return {url, method: init?.method, body: init?.body === undefined ? undefined : JSON.parse(String(init.body))};
};

const WRITTEN = {done: null, files: ["time/pat/2026-11.timedot"], retried: false, included: null};

describe("UNIT time entries API", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("lists with the query as given", async () => {
        const fetch = answer(200, list([entry()]));
        const got = await fetchEntries("from=2026-11-01&to=2026-11-30");
        expect(got.entries[0].account).toBe("clients:acme:advisory");
        expect(call(fetch)).toMatchObject({url: "/api/time/entries?from=2026-11-01&to=2026-11-30", method: "GET"});
        await fetchEntries("");
        expect(call(fetch, 1).url).toBe("/api/time/entries");
    });

    it("refuses a list that isn't one", async () => {
        answer(200, {entries: [{id: 1}], total: null});
        await expect(fetchEntries("")).rejects.toThrow(/shape/);
        expect(isEntryList(list([entry()]))).toBe(true);
    });

    it("adds, edits, deletes and acts in bulk", async () => {
        const fetch = answer(200, WRITTEN);
        await addEntry({date: "2026-11-27", account: "internal:admin", duration: "1h30", note: "expense reports"});
        await editEntry({id: "time/pat/2026-11.timedot@20", note: "call"});
        await deleteEntry("time/pat/2026-11.timedot@20");
        await bulkEntries(["a@1", "a@2"], {op: "nobill", reason: "courtesy"});
        expect(call(fetch, 0)).toEqual({
            url: "/api/time/entries",
            method: "POST",
            body: {date: "2026-11-27", account: "internal:admin", duration: "1h30", note: "expense reports"},
        });
        expect(call(fetch, 1)).toMatchObject({method: "PUT", body: {id: "time/pat/2026-11.timedot@20", note: "call"}});
        expect(call(fetch, 2)).toMatchObject({url: "/api/time/entries?id=time%2Fpat%2F2026-11.timedot%4020", method: "DELETE"});
        expect(call(fetch, 3)).toMatchObject({url: "/api/time/entries/bulk", body: {ids: ["a@1", "a@2"], op: "nobill", reason: "courtesy"}});
    });

    it("a refusal keeps its code and the server's suggestion", async () => {
        answer(422, {error: "hledger reads `re:` in a note as a tag", code: "note-tag", offending: "re:", suggestion: "re K-1"});
        const outcome = await addEntry({date: "2026-11-27", account: "internal:admin", duration: "1h", note: "re: K-1"}).then(
            () => null,
            (e: unknown) => refusal(e)
        );
        expect(outcome).toMatchObject({ok: false, code: "note-tag", body: {suggestion: "re K-1"}});
        expect(refusal(new Error("network down"))).toMatchObject({ok: false, code: "unreachable"});
    });
});

describe("UNIT time accounts API", () => {
    afterEach(() => vi.unstubAllGlobals());

    it("reads the tree, for a month", async () => {
        const fetch = answer(200, accountsView());
        const view = await fetchAccounts("2026-10");
        expect(view.undeclared).toEqual(["internal:training"]);
        expect(call(fetch).url).toBe("/api/time/accounts?month=2026-10");
        await fetchAccounts();
        expect(call(fetch, 1).url).toBe("/api/time/accounts");
        expect(isAccountsView({accounts: [{account: "x"}], undeclared: [], file: ""})).toBe(false);
    });

    it("declares, sets a tag, archives and renames", async () => {
        const fetch = answer(200, {...WRITTEN, done: {from: "internal:admin", to: "internal:office", lines: [], files: [], applied: false}});
        await declareAccount("clients:contoso");
        await setAccountTag("clients:initech", "item", "ARCH");
        await setAccountTag("clients:initech", "billable", null);
        await archiveAccount("internal:training", true);
        const preview = await renameAccount("internal:admin", "internal:office", false);
        expect(preview.done.applied).toBe(false);
        expect(call(fetch, 0)).toMatchObject({url: "/api/time/accounts/declare", body: {account: "clients:contoso", tags: {}}});
        expect(call(fetch, 1)).toMatchObject({url: "/api/time/accounts/set-tag", body: {account: "clients:initech", tag: "item", value: "ARCH"}});
        expect(call(fetch, 2).body).toEqual({account: "clients:initech", tag: "billable", value: null});
        expect(call(fetch, 3)).toMatchObject({url: "/api/time/accounts/archive", body: {account: "internal:training", archived: true}});
        expect(call(fetch, 4)).toMatchObject({url: "/api/time/accounts/rename", body: {from: "internal:admin", to: "internal:office", apply: false}});
    });
});
