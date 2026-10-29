import {describe, expect, it} from "vitest";
import {ApiShapeError} from "#lib/api/client.js";
import {AUTO_RULES_LISTING as LISTING} from "#lib/testing/autoRow.js";
import {decodeAutoRules} from "./decode";

describe("UNIT decodeAutoRules", () => {
    it("decodes the listing", () => {
        const listing = decodeAutoRules(LISTING);
        expect(listing.inForce).toBe(true);
        expect(listing.conf).toBe("hledger.conf");
        expect(listing.rules[0]).toMatchObject({index: 0, file: "rules.journal", line: 5, generated: 4, unsupported: null});
        expect(listing.rules[0].problems[0]).toMatchObject({kind: "unknownAccount", txnIndex: null, account: "liabilities:tax:sales"});
    });

    it("decodes what the editor needs: each rule's file, block, revision and form, and where a new rule goes", () => {
        const listing = decodeAutoRules(LISTING);
        expect(listing.editable).toBe(true);
        expect(listing.newRule).toEqual({journalId: "main.journal", revision: "rev-main"});
        expect(listing.rules[0]).toMatchObject({journalId: "rules.journal", block: 0, revision: "rev-rules"});
        expect(listing.rules[0].form).toMatchObject({matchAccount: "revenue:sales", counter: "assets:checking", forecastOnly: false});
        expect(listing.rules[0].form?.postings[0].percent).toEqual({m: 10n, p: 0});
        expect(listing.rules[1].form).toBeNull();
        expect(listing.rules[2]).toMatchObject({journalId: null, revision: null});
        expect(decodeAutoRules({...LISTING, newRule: null}).newRule).toBeNull();
    });

    it("refuses a listing without the editor's fields", () => {
        const without = <T extends object>(value: T, key: keyof T): Record<string, unknown> => {
            const copy: Record<string, unknown> = {...(value as Record<string, unknown>)};
            delete copy[key as string];
            return copy;
        };
        expect(() => decodeAutoRules(without(LISTING, "editable"))).toThrow(ApiShapeError);
        expect(() => decodeAutoRules({...LISTING, rules: [without(LISTING.rules[0], "block")]})).toThrow(ApiShapeError);
        const form = without(LISTING.rules[0].form!, "forecastOnly");
        expect(() => decodeAutoRules({...LISTING, rules: [{...LISTING.rules[0], form}]})).toThrow(ApiShapeError);
    });

    it("reads a missing config file as null", () => {
        expect(decodeAutoRules({...LISTING, conf: null, inForce: false}).conf).toBeNull();
    });

    it("refuses a drifted shape instead of rendering an empty list", () => {
        expect(() => decodeAutoRules({...LISTING, rules: undefined})).toThrow(ApiShapeError);
        expect(() => decodeAutoRules({...LISTING, inForce: "yes"})).toThrow(ApiShapeError);
        const rule = {...LISTING.rules[0], problems: [{...LISTING.rules[0].problems[0], kind: "typo"}]};
        expect(() => decodeAutoRules({...LISTING, rules: [rule]})).toThrow(ApiShapeError);
        expect(() => decodeAutoRules({...LISTING, rules: [{...LISTING.rules[0], generated: -1}]})).toThrow(ApiShapeError);
    });
});
