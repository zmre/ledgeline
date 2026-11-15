import {describe, expect, it} from "vitest";
import {ApiShapeError} from "#lib/api/client.js";
import {decodeConnectResult, decodeLinkPreviews, decodeManualAccounts, decodeSharePreview, decodeSyncRun, decodeSyncStatus} from "./decode";
import {chaseRemote, fixtureManual, fixturePreviews, fixtureRun, fixtureSharePreview, fixtureStatus, CONNECTION_ID} from "./fixtures";

/** Through JSON and back, as the wire would carry it. */
const wire = (value: unknown): unknown => JSON.parse(JSON.stringify(value));

describe("UNIT sync decode", () => {
    it("round-trips every fixture", () => {
        expect(decodeSyncStatus(wire(fixtureStatus()))).toEqual(fixtureStatus());
        expect(decodeSyncRun(wire(fixtureRun()))).toEqual(fixtureRun());
        expect(decodeLinkPreviews(wire({accounts: fixturePreviews()}))).toEqual(fixturePreviews());
        expect(decodeSharePreview(wire(fixtureSharePreview()))).toEqual(fixtureSharePreview());
        expect(decodeManualAccounts(wire({accounts: fixtureManual()}))).toEqual(fixtureManual());
        expect(decodeConnectResult(wire({connectionId: CONNECTION_ID, accounts: [chaseRemote()]}))).toEqual({
            connectionId: CONNECTION_ID,
            accounts: [chaseRemote()],
        });
    });

    it("keeps the requested uncategorized count when the server sends it", () => {
        const run = fixtureRun();
        run.accounts[0].uncategorized = 2;
        expect(decodeSyncRun(wire(run)).accounts[0].uncategorized).toBe(2);
    });

    it("names the field that broke", () => {
        const status = wire(fixtureStatus()) as {connections: {state: string}[]};
        status.connections[0].state = "fine";
        expect(() => decodeSyncStatus(status)).toThrow(ApiShapeError);
        expect(() => decodeSyncStatus(status)).toThrow(/state/);
        expect(() => decodeSyncStatus({...fixtureStatus(), accounts: null})).toThrow(/accounts/);
        expect(() => decodeSyncStatus(null)).toThrow(ApiShapeError);
    });
});
