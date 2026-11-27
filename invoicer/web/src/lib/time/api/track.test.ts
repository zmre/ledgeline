// The tracker's API decoding: the state's shape check, the account names
// Other… offers (archived ones left out), and today's lines for the card.
import {describe, expect, it} from "vitest";
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
        const body = {
            accounts: [
                {account: "clients:acme", archived: null},
                {account: "internal:old", archived: {value: "yes", from: "internal:old", inherited: false}},
                {account: "internal:old:sub", archived: {value: "yes", from: "internal:old", inherited: true}},
                {account: "internal:admin"},
                {nope: 1},
            ],
        };
        expect(accountNamesOf(body)).toEqual(["clients:acme", "internal:admin"]);
        expect(accountNamesOf({})).toEqual([]);
    });

    it("reads today's lines as account, seconds and note", () => {
        const body = {
            entries: [
                {account: "clients:acme:advisory", quantity: {seconds: 3600, hours: "1", text: "1h"}, note: "call"},
                {account: "internal:admin", quantity: {seconds: 900}, note: null},
                {account: "broken"},
            ],
        };
        expect(dayLinesOf(body)).toEqual([
            {account: "clients:acme:advisory", seconds: 3600, note: "call"},
            {account: "internal:admin", seconds: 900, note: ""},
        ]);
    });
});
