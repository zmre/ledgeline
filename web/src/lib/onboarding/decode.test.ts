import {ApiShapeError} from "#lib/api/client.js";
import {describe, expect, it} from "vitest";
import {decodeCreatedLedger, decodeNewLedgerStatus, decodePickedFolder, decodeTemplate} from "./decode";

describe("UNIT onboarding decode", () => {
    it("reads the status", () => {
        expect(decodeNewLedgerStatus({desktop: true, journalOpen: false})).toEqual({desktop: true, journalOpen: false});
        expect(() => decodeNewLedgerStatus({desktop: "yes"})).toThrow(ApiShapeError);
        expect(() => decodeNewLedgerStatus(null)).toThrow(ApiShapeError);
    });

    it("reads a template's accounts and tags", () => {
        const accounts = decodeTemplate({
            accounts: [
                {name: "assets", tags: [{name: "type", value: "A"}]},
                {name: "transfers", tags: [{name: "clearing", value: ""}]},
            ],
        });
        expect(accounts).toEqual([
            {name: "assets", tags: [{name: "type", value: "A"}]},
            {name: "transfers", tags: [{name: "clearing", value: ""}]},
        ]);
        expect(Object.isFrozen(accounts)).toBe(true);
        expect(() => decodeTemplate({accounts: [{name: "x"}]})).toThrow(ApiShapeError);
    });

    it("reads a picked folder, and a cancelled picker as null", () => {
        expect(decodePickedFolder({folder: {id: "abc", name: "Books", empty: true}})).toEqual({id: "abc", name: "Books", empty: true});
        expect(decodePickedFolder({folder: null})).toBeNull();
        expect(() => decodePickedFolder({folder: {id: 1}})).toThrow(ApiShapeError);
    });

    it("reads what was created", () => {
        expect(decodeCreatedLedger({folder: "Books", files: ["main.journal", "accounts.journal"]})).toEqual({
            folder: "Books",
            files: ["main.journal", "accounts.journal"],
        });
        expect(() => decodeCreatedLedger({folder: "Books"})).toThrow(ApiShapeError);
    });
});
