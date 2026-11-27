import {describe, expect, it} from "vitest";
import {
    accountRegex,
    billableCommand,
    chartCommand,
    dailyCommand,
    notesCommand,
    periodArgs,
    shellWord,
    topCustomersCommand,
    totalCommand,
    treeCommand,
    unbilledCommand,
} from "./hledger.js";
import {DEFAULT_QUERY} from "./query.js";

const F = "time.journal";
const NOV = {from: "2026-11-01", to: "2026-11-30"};

describe("UNIT time dashboard hledger commands", () => {
    it("names a whole month with -p, else -b and an exclusive -e", () => {
        expect(periodArgs(NOV)).toEqual(["-p", "2026-11"]);
        expect(periodArgs({from: "2026-11-01", to: "2026-11-26"})).toEqual(["-b", "2026-11-01", "-e", "2026-11-27"]);
        expect(periodArgs({from: "2026-10-01", to: "2026-11-30"})).toEqual(["-b", "2026-10-01", "-e", "2026-12-01"]);
    });

    it("quotes what the shell would split", () => {
        expect(shellWord("tag:customer=Acme Corp")).toBe("'tag:customer=Acme Corp'");
        expect(shellWord("it's")).toBe("'it'\\''s'");
        expect(shellWord("--depth")).toBe("--depth");
        expect(accountRegex("clients:acme")).toBe("^clients:acme(:|$)");
    });

    it("reproduces each figure", () => {
        expect(totalCommand(F, NOV, DEFAULT_QUERY).text).toBe("hledger -f time.journal bal -p 2026-11 --depth 0");
        expect(billableCommand(F, NOV, DEFAULT_QUERY).text).toBe("hledger -f time.journal bal -p 2026-11 --pivot billable");
        expect(dailyCommand(F, NOV, DEFAULT_QUERY).text).toBe("hledger -f time.journal bal -p 2026-11 -D --depth 0");
        expect(unbilledCommand(F, NOV, DEFAULT_QUERY).text).toBe(
            "hledger -f time.journal bal -p 2026-11 --pivot billable '^yes$' tag:customer not:tag:invoice not:tag:nobill"
        );
        expect(treeCommand(F, NOV, DEFAULT_QUERY, 2).text).toBe("hledger -f time.journal bal -p 2026-11 --tree --depth 2");
        expect(chartCommand(F, NOV, DEFAULT_QUERY, "week", "branch").text).toBe("hledger -f time.journal bal -p 2026-11 -W --depth 1");
        expect(chartCommand(F, NOV, DEFAULT_QUERY, "day", "billable").text).toBe("hledger -f time.journal bal -p 2026-11 -D --pivot billable");
        expect(topCustomersCommand(F, NOV, DEFAULT_QUERY).text).toBe("hledger -f time.journal bal -p 2026-11 --pivot customer -S");
        expect(notesCommand(F, NOV, DEFAULT_QUERY)).toMatchObject({
            text: "hledger -f time.journal print -p 2026-11",
            caveat: expect.stringContaining("can't group by note"),
        });
    });

    it("carries the filters, the billable one as a pivot where it can be exact", () => {
        const q = {...DEFAULT_QUERY, customer: "Acme Corp", account: "clients:acme", billable: "yes" as const};
        expect(totalCommand(F, NOV, q)).toEqual({
            text: "hledger -f time.journal bal -p 2026-11 '^clients:acme(:|$)' 'tag:customer=Acme Corp' --pivot billable '^yes$' --depth 0",
        });
        const tree = treeCommand(F, NOV, q, 3);
        expect(tree.text).toBe("hledger -f time.journal bal -p 2026-11 '^clients:acme(:|$)' 'tag:customer=Acme Corp' tag:billable=yes --tree --depth 3");
        expect(tree.caveat).toContain("billable: no under a billable one");
        // The billable figure under the billable filter: one pivot, not two.
        expect(billableCommand(F, NOV, {...DEFAULT_QUERY, billable: "no"}).text).toBe("hledger -f time.journal bal -p 2026-11 --pivot billable '^no$'");
    });

    it("flags person filters, which hledger sees only as tags", () => {
        expect(totalCommand(F, NOV, {...DEFAULT_QUERY, person: "sam"})).toMatchObject({
            text: expect.stringContaining("tag:person=sam"),
            caveat: expect.stringContaining("person: tag"),
        });
        expect(totalCommand(F, NOV, {...DEFAULT_QUERY, person: "-"}).text).toContain("not:tag:person");
    });

    it("says billing states as tags where it can, and flags the rest and the search", () => {
        expect(totalCommand(F, NOV, {...DEFAULT_QUERY, billed: "invoiced"})).toEqual({text: "hledger -f time.journal bal -p 2026-11 tag:invoice --depth 0"});
        expect(totalCommand(F, NOV, {...DEFAULT_QUERY, billed: "unbilled"}).caveat).toContain("billing filter");
        expect(totalCommand(F, NOV, {...DEFAULT_QUERY, q: "email"})).toMatchObject({
            text: "hledger -f time.journal bal -p 2026-11 --depth 0",
            caveat: expect.stringContaining("leaves out the search"),
        });
    });
});
