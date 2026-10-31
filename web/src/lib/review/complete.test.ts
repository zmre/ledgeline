// Description and tag completion (plan 32 D15).

import {describe, expect, it} from "vitest";
import {parseTag, payeeWeight, rankPayees, tagCompletions} from "./complete";
import type {PayeeUse, TagUse} from "./types";

const TODAY = "2026-10-30";
const PAYEES: PayeeUse[] = [
    {payee: "Netflix", count: 6, last: "2026-09-14"},
    {payee: "SQ *BLUE BOTTLE 4412 OAKLAND", count: 1, last: "2026-10-12"},
    {payee: "Blue Bottle Coffee", count: 1, last: "2026-09-01"},
    {payee: "Bluebird Books", count: 9, last: "2023-01-01"},
    {payee: "Safeway", count: 4, last: "2026-10-25"},
];

describe("UNIT rankPayees", () => {
    it("ranks prefixes, then word starts, each by frequency × recency", () => {
        expect(rankPayees("blue", PAYEES, TODAY).map((use) => use.payee)).toEqual(["Blue Bottle Coffee", "Bluebird Books", "SQ *BLUE BOTTLE 4412 OAKLAND"]);
    });

    it("lets recency outweigh an old, larger count", () => {
        expect(payeeWeight({payee: "a", count: 2, last: TODAY}, TODAY)).toBeGreaterThan(payeeWeight({payee: "b", count: 9, last: "2023-01-01"}, TODAY));
    });

    it("lists the heaviest payees for an empty query, and does not offer back the exact text", () => {
        expect(rankPayees("", PAYEES, TODAY, 2).map((use) => use.payee)).toEqual(["Netflix", "Safeway"]);
        expect(rankPayees("safeway", PAYEES, TODAY)).toEqual([]);
        expect(rankPayees("ottle", PAYEES, TODAY)).toEqual([]);
    });
});

describe("UNIT tag completion", () => {
    const TAGS: TagUse[] = [
        {
            name: "reviewed",
            count: 5,
            values: [
                {value: "ignore", count: 1},
                {value: "2026-09-02", count: 3},
            ],
        },
        {name: "receipt", count: 1, values: [{value: "0042", count: 1}]},
        {name: "trip", count: 8, values: []},
    ];

    it("completes the name, most used first, leaving out names on the card", () => {
        expect(tagCompletions("re", TAGS).map((c) => c.insert)).toEqual(["reviewed:", "receipt:"]);
        expect(tagCompletions("", TAGS, new Set(["trip"])).map((c) => c.label)).toEqual(["reviewed", "receipt"]);
    });

    it("completes the value after the colon", () => {
        expect(tagCompletions("reviewed:", TAGS).map((c) => c.insert)).toEqual(["reviewed:2026-09-02", "reviewed:ignore"]);
        expect(tagCompletions("reviewed:ig", TAGS).map((c) => c.kind)).toEqual(["value"]);
        expect(tagCompletions("nope:x", TAGS)).toEqual([]);
    });

    it("parses only tags the engine can write", () => {
        expect(parseTag(" trip : paris ")).toEqual(["trip", "paris"]);
        expect(parseTag("flag")).toEqual(["flag", ""]);
        expect(parseTag("two words:x")).toBeNull();
        expect(parseTag("a:b,c")).toBeNull();
        expect(parseTag(":x")).toBeNull();
    });
});
