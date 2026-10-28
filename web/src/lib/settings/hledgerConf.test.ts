import {decodeHledgerConf} from "#lib/api/nativeDecode.js";
import {ApiShapeError} from "#lib/api/client.js";
import {hledgerConfBody} from "#lib/testing/hledgerConfFixture.js";
import {describe, expect, it} from "vitest";
import {blankOverride, isDirty, regexProblem, toForm, toSaveBody, validateForm} from "./hledgerConfModel";

describe("UNIT decodeHledgerConf", () => {
    it("decodes every field", () => {
        const conf = decodeHledgerConf(hledgerConfBody());
        expect(conf.handle).toBe("hledger.conf");
        expect(conf.origin).toBe("ledger");
        expect(conf.depth.general).toEqual({all: 3, perRegex: []});
        expect(conf.depth.balancesheet.perRegex).toEqual([{regex: "^assets:inv", depth: 3}]);
        expect(conf.effectiveDepth.cashflow.all).toBe(3);
        expect(Object.isFrozen(conf)).toBe(true);
    });

    it("reads the no-file shape", () => {
        const conf = decodeHledgerConf(hledgerConfBody({handle: null, origin: null, createHandle: "hledger.conf", revision: ""}));
        expect(conf.handle).toBeNull();
        expect(conf.createHandle).toBe("hledger.conf");
    });

    it("refuses a missing section rather than reading it as unset", () => {
        const body = hledgerConfBody();
        delete (body.depth as Record<string, unknown>).cashflow;
        expect(() => decodeHledgerConf(body)).toThrow(ApiShapeError);
        expect(() => decodeHledgerConf(hledgerConfBody({origin: "elsewhere"}))).toThrow(ApiShapeError);
        expect(() => decodeHledgerConf(hledgerConfBody({auto: "yes"}))).toThrow(ApiShapeError);
    });
});

describe("UNIT hledgerConfModel", () => {
    const conf = decodeHledgerConf(hledgerConfBody());

    it("round-trips: an untouched form is clean and saves nothing", () => {
        const form = toForm(conf);
        expect(form.sections.general.all).toBe("3");
        expect(isDirty(conf, form)).toBe(false);
        expect(toSaveBody(conf, form)).toEqual({revision: "rev-1"});
    });

    it("sends only what changed", () => {
        const form = toForm(conf);
        form.auto = true;
        form.sections.cashflow = {all: "2", overrides: [{regex: " assets ", depth: "4"}]};
        form.sections.general = {all: "", overrides: []};
        expect(toSaveBody(conf, form)).toEqual({
            revision: "rev-1",
            auto: true,
            depth: {general: {all: null, perRegex: []}, cashflow: {all: 2, perRegex: [{regex: "assets", depth: 4}]}},
        });
    });

    it("names every problem before a round trip", () => {
        const form = toForm(conf);
        form.sections.general.all = "deep";
        form.sections.balance.overrides = [
            {regex: "a b", depth: "1"},
            {regex: "x", depth: ""},
            {regex: "x", depth: "2"},
        ];
        form.sections.incomestatement.overrides = [blankOverride()];
        const errors = validateForm(form);
        expect(errors).toHaveLength(6);
        expect(errors[0]).toContain("Every report");
        expect(errors.some((e) => e.includes("listed twice"))).toBe(true);
    });

    it("mirrors the engine's regex refusals", () => {
        for (const bad of ["", "a b", "a#b", "-a", "a=b", "("]) expect(regexProblem(bad), bad).not.toBeNull();
        expect(regexProblem("^assets:inv(:|$)")).toBeNull();
    });
});
