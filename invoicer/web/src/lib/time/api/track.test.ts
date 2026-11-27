// The tracker's API decoding: the state's shape check, the account names
// Other… offers (archived ones left out), and today's lines for the card.
import {describe, expect, it} from "vitest";
import {accountRow, accountsView, entry, list, qty} from "#lib/time/testing.js";
import {trackState} from "#lib/time/track/fixtures.js";
import {accountNamesOf, dayLinesOf, isTrackState} from "./track.js";

describe("UNIT the tracker's API answers", () => {
    it("knows the state when it sees it", () => {
        expect(isTrackState(trackState())).toBe(true);
        expect(isTrackState({configured: false, slots: [], week: [], step: 5, today_total: {seconds: 0}})).toBe(true);
        expect(isTrackState({configured: true})).toBe(false);
        expect(isTrackState(null)).toBe(false);
    });

    it("lists the accounts, leaving archived ones out", () => {
        const archived = {value: "yes", from: "internal:old", inherited: false};
        const view = accountsView({
            accounts: [
                accountRow({account: "clients:acme"}),
                accountRow({account: "internal:old", archived}),
                accountRow({account: "internal:old:sub", archived: {...archived, inherited: true}}),
                accountRow({account: "internal:admin", archived: {...archived, value: "no"}}),
            ],
        });
        expect(accountNamesOf(view)).toEqual(["clients:acme", "internal:admin"]);
        expect(accountNamesOf({accounts: []})).toEqual([]);
    });

    it("reads today's lines as account, seconds and note", () => {
        const lines = list([entry({quantity: qty(60), note: "call"}), entry({account: "internal:admin", quantity: qty(15), note: ""})]);
        expect(dayLinesOf(lines)).toEqual([
            {account: "clients:acme:advisory", seconds: 3600, note: "call"},
            {account: "internal:admin", seconds: 900, note: ""},
        ]);
    });
});
