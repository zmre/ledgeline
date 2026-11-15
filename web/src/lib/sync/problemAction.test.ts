import {describe, expect, it} from "vitest";
import {CONNECTION_ID, conAuthProblem, fixtureStatus, heldProblem, revokedProblem} from "./fixtures";
import {docUrl, placedProblems, problemButtons, safeUrl, whoLabel, worstSeverity} from "./problemAction";
import type {WireProblem} from "./types";

const scope = {connectionId: CONNECTION_ID, accountKey: "f6e5d4c3b2"};

describe("UNIT sync problemAction", () => {
    it("opens SimpleFIN for a sign-in problem and re-syncs the account for I've fixed it", () => {
        const {primary, secondary} = problemButtons(conAuthProblem(), scope);
        expect(primary).toEqual({label: "Fix at SimpleFIN ↗", effect: {type: "url", url: "https://beta-bridge.simplefin.org/"}});
        expect(secondary).toEqual({label: "I've fixed it", effect: {type: "sync", connectionId: CONNECTION_ID, accountKey: "f6e5d4c3b2"}});
    });

    it("drops a link that is not https", () => {
        const problem: WireProblem = {...conAuthProblem(), action: {kind: "open-url", label: "Fix", url: "javascript:alert(1)"}};
        expect(problemButtons(problem, scope).primary).toBeNull();
        expect(safeUrl("http://example.com/")).toBeNull();
        expect(safeUrl("https://user:pw@example.com/")).toBeNull();
        expect(safeUrl("https://chase.com")).toBe("https://chase.com/");
        expect(safeUrl(undefined)).toBeNull();
    });

    it("maps every other action to its effect, preferring the server's ref", () => {
        expect(problemButtons(revokedProblem(), {connectionId: null, accountKey: null}).primary?.effect).toEqual({
            type: "reconnect",
            connectionId: CONNECTION_ID,
        });
        expect(problemButtons(heldProblem("k1"), scope).primary?.effect).toEqual({type: "review-held", accountKey: "k1"});
        const kinds: [NonNullable<WireProblem["action"]>["kind"], string][] = [
            ["retry", "sync"],
            ["recheck", "sync"],
            ["edit-rules", "edit-rules"],
            ["choose-file", "choose-file"],
            ["connect", "connect"],
            ["set-hledger", "set-hledger"],
            ["show-balance", "show-balance"],
        ];
        for (const [kind, type] of kinds) {
            const problem: WireProblem = {...conAuthProblem(), action: {kind, label: "Go"}};
            expect(problemButtons(problem, scope).primary?.effect.type).toBe(type);
        }
        expect(problemButtons({...conAuthProblem(), action: null, secondary: null}, scope)).toEqual({primary: null, secondary: null});
    });

    it("labels who has to act and links the docs", () => {
        expect(whoLabel("bank")).toBe("Your bank");
        expect(whoLabel("simplefin")).toBe("SimpleFIN");
        expect(docUrl("con-auth")).toBe("https://zmre.github.io/ledgeline/imports/bank-sync/#con-auth");
    });

    it("finds the worst severity", () => {
        expect(worstSeverity([])).toBeNull();
        expect(worstSeverity([conAuthProblem(), revokedProblem()])).toBe("error");
        expect(worstSeverity([conAuthProblem()])).toBe("warning");
    });

    it("lists every problem with where it is, worst first", () => {
        const status = fixtureStatus();
        status.connections[0].problems = [revokedProblem()];
        const placed = placedProblems(status);
        expect(placed.map((p) => [p.problem.code, p.where])).toEqual([
            ["revoked", "SimpleFIN Bridge"],
            ["held", "First Tech · Checking ••1234"],
            ["con-auth", "Chase · Sapphire ••9921"],
        ]);
        expect(placed[1].scope).toEqual({connectionId: CONNECTION_ID, accountKey: "a1b2c3d4e5"});
        expect(new Set(placed.map((p) => p.key)).size).toBe(3);
    });

    it("leaves info out, and scopes an account of an unknown connection to no connection", () => {
        const status = fixtureStatus();
        status.connections = [];
        status.accounts[0].problems = [{...heldProblem(), severity: "info"}];
        const placed = placedProblems(status);
        expect(placed).toHaveLength(1);
        expect(placed[0].scope.connectionId).toBeNull();
    });
});
