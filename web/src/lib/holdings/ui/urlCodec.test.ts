import {describe, expect, it} from "vitest";
import type {HoldingsScope} from "$lib/holdings/types";
import {scopeToSearch, searchToScope} from "./urlCodec";

const TODAY = "2026-07-08";

function scope(overrides: Partial<HoldingsScope> = {}): HoldingsScope {
    return {accounts: new Set<string>(), mode: "include", asOf: TODAY, gainPeriod: "all", ...overrides};
}

describe("UNIT holdings urlCodec", () => {
    describe("scopeToSearch", () => {
        it("serializes the fresh-visit default to an empty string", () => {
            expect(scopeToSearch(scope(), TODAY)).toBe("");
        });

        it("writes asof only when it differs from today (never a remembered date)", () => {
            expect(scopeToSearch(scope({asOf: "2025-01-01"}), TODAY)).toBe("asof=2025-01-01");
        });

        it("writes accounts sorted and mode only when exclude", () => {
            const s = scope({accounts: new Set(["expenses", "assets:broker"]), mode: "exclude"});
            expect(scopeToSearch(s, TODAY)).toBe("acct=assets%253Abroker%2Cexpenses&mode=exclude");
        });

        it("writes gain only when the window isn't all-time", () => {
            expect(scopeToSearch(scope({gainPeriod: "all"}), TODAY)).toBe("");
            expect(scopeToSearch(scope({gainPeriod: "ytd"}), TODAY)).toBe("gain=ytd");
            expect(scopeToSearch(scope({gainPeriod: "12mo"}), TODAY)).toBe("gain=12mo");
        });

        it("round-trips account names containing commas", () => {
            const s = scope({accounts: new Set(["assets:a,b", "assets:c"])});
            const parsed = searchToScope(scopeToSearch(s, TODAY), TODAY);
            expect([...parsed.accounts].sort()).toEqual(["assets:a,b", "assets:c"]);
        });
    });

    describe("searchToScope", () => {
        it("absent params always mean today/empty/include", () => {
            expect(searchToScope("", TODAY)).toEqual(scope());
            expect(searchToScope("?", TODAY)).toEqual(scope());
        });

        it("parses a full query with or without the leading question mark", () => {
            const expected = scope({asOf: "2025-01-01", accounts: new Set(["assets:broker"]), mode: "exclude"});
            expect(searchToScope("?asof=2025-01-01&acct=assets%3Abroker&mode=exclude", TODAY)).toEqual(expected);
            expect(searchToScope("asof=2025-01-01&acct=assets%3Abroker&mode=exclude", TODAY)).toEqual(expected);
        });

        it("falls back to today on malformed asof and include on unknown mode", () => {
            expect(searchToScope("?asof=notadate&mode=banana", TODAY)).toEqual(scope());
        });

        it("parses gain and falls back to all-time on an unknown window", () => {
            expect(searchToScope("?gain=ytd", TODAY)).toEqual(scope({gainPeriod: "ytd"}));
            expect(searchToScope("?gain=12mo", TODAY)).toEqual(scope({gainPeriod: "12mo"}));
            expect(searchToScope("?gain=banana", TODAY)).toEqual(scope());
        });

        it("ignores empty account segments", () => {
            expect([...searchToScope("?acct=a,,b", TODAY).accounts].sort()).toEqual(["a", "b"]);
        });

        it("round-trips a non-default scope", () => {
            const s = scope({asOf: "2024-12-31", accounts: new Set(["assets:broker:taxable:vti"]), mode: "exclude", gainPeriod: "ytd"});
            expect(searchToScope(scopeToSearch(s, TODAY), TODAY)).toEqual(s);
        });
    });
});
