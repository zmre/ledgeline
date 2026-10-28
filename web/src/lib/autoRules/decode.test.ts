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
