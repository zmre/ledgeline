import {describe, expect, it} from "vitest";
import {ApiTimeoutError, ApiUnreachableError} from "./client";
import {classify} from "./editFailure";
import {EngineRefusalError} from "./native";

describe("UNIT editFailure — classify", () => {
    // The whole point of EngineRefusalError (native.ts): a journal typo
    // surfaced on a READ — a 400 from the refetch after an edit, or from a
    // report route — is the same fact as a write-path ValidationError. Before
    // this mapping it fell through to ApiUnreachableError's "network" kind,
    // which sent the user to check their connection instead of their journal.
    it("maps a read-path engine refusal to `validation`, never to `network`", () => {
        const sentence = "account 'assets:x' declares `holdings: y`, which is not one of stocks, other, none";
        expect(classify(new EngineRefusalError(sentence))).toEqual({kind: "validation", message: sentence});
    });

    it("still maps genuine connectivity failures (and timeouts, their subclass) to `network`", () => {
        expect(classify(new ApiUnreachableError("Cannot reach the Ledgeline engine")).kind).toBe("network");
        expect(classify(new ApiTimeoutError("GET /api/journal timed out after 30s")).kind).toBe("network");
    });
});
