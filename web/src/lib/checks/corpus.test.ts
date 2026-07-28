// The local `unbalanced` rule against the hledger corpus (FE-3).
//
// Every journal under fixtures/corpus/ (errors/ excluded) is one hledger 1.52
// ACCEPTS, and each has its `hledger print -O json` output captured beside it —
// the same wire shape the SPA fetches. So this rule, which is the only balance
// check running against a plain hledger-web, must find nothing in any of them.
// It found a red `error` badge on every journal using `(virtual)` postings
// until the two balancing groups were separated.

import {readdirSync, readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {normalizeTransactions} from "../api/normalize";
import {runChecks, type Problem} from "./engine";

const CORPUS = fileURLToPath(new URL("../../../../fixtures/corpus/", import.meta.url));

/**
 * The one corpus case this rule cannot decide, and does not have to: hledger
 * balances `55.3653 C @ 30.92189512 D` against `-1712 D` because the leftover
 * is under its `amountLooksZero` tolerance, while this rule is exact. That is
 * one of the two shapes `CheckContext.engineChecked` exists for — the engine
 * reproduces hledger's tolerance, so wherever it answered, this rule stands
 * down. Named rather than silently filtered, and its finding is asserted below,
 * so the exemption cannot quietly widen.
 */
const TOLERANCE_CASE = "precision";
const TOLERANCE_RESIDUE = "D -0.000000112664 remaining";

const cases = readdirSync(CORPUS)
    .filter((name) => name.endsWith(".print.json"))
    .map((name) => [name.replace(".print.json", ""), name] as const)
    .sort();

const unbalancedIn = (file: string): Problem[] => {
    const txns = normalizeTransactions(JSON.parse(readFileSync(CORPUS + file, "utf8")) as unknown);
    return runChecks(txns, {prices: []}).filter((p) => p.rule === "unbalanced");
};

describe("UNIT checks/rules unbalanced over the hledger corpus", () => {
    it("covers the whole corpus", () => {
        expect(cases.length).toBeGreaterThan(30);
        expect(cases.some(([base]) => base === TOLERANCE_CASE)).toBe(true); // no stale exemption
    });

    it.each(cases.filter(([base]) => base !== TOLERANCE_CASE))("finds nothing unbalanced in %s", (_base, file) => {
        expect(unbalancedIn(file)).toEqual([]);
    });

    it(`reports only the sub-tolerance cost residual in ${TOLERANCE_CASE}`, () => {
        const problems = unbalancedIn(`${TOLERANCE_CASE}.print.json`);
        expect(problems).toHaveLength(1);
        expect(problems[0].message).toBe(`postings do not sum to zero: ${TOLERANCE_RESIDUE}`);
        // …and nothing at all once the engine has answered.
        const txns = normalizeTransactions(JSON.parse(readFileSync(`${CORPUS}${TOLERANCE_CASE}.print.json`, "utf8")) as unknown);
        expect(runChecks(txns, {prices: [], engineChecked: true}).filter((p) => p.rule === "unbalanced")).toEqual([]);
    });

    it("finds nothing unbalanced in the virtual-posting journals specifically", () => {
        // The FE-3 repro, kept as a named case so the intent survives a corpus edit.
        for (const file of ["virtual-postings.print.json", "balanced-virtual-postings.print.json"]) {
            const txns = normalizeTransactions(JSON.parse(readFileSync(CORPUS + file, "utf8")) as unknown);
            expect(txns.some((t) => t.postings.some((p) => p.type !== undefined))).toBe(true); // the fixture really does carry ptype
            expect(runChecks(txns, {prices: []}).filter((p) => p.severity === "error")).toEqual([]);
        }
    });
});
