// The hledger command behind each figure on the time dashboard (plan 46
// T-dash, as plan 37's KPIs do): run against the roll-up, it prints the same
// number. Checked by hand against invoicer/fixtures/time with hledger 1.52.
//
// Two things hledger can't say exactly, which the commands then flag:
//   - Billable is nearest-ancestor-wins (`clients:acme:travel  ; billable:
//     no` under `clients  ; billable: yes`). `--pivot billable` resolves it
//     that way, but a `tag:billable=…` query matches the tag on ANY
//     ancestor, so the override counts on both sides. The billable filter
//     uses the pivot where the command has no other pivot and no account
//     tree; elsewhere it falls back to the query, with a caveat.
//   - hledger knows a person only from a `person:` tag; time the Invoicer
//     attributes by its folder (`time/sam/…`) carries none.
// The search (`q`, text in the note or account) and the billing states
// other than "on an invoice" and "written off" have no hledger query of
// their own here; the commands leave them out and say so.
import {addDays, isMonthEnd, isMonthStart} from "../dates.js";
import {UNATTRIBUTED, type Range} from "../filters/query.js";
import type {DashQuery, Grain, Split} from "./query.js";

export interface Command {
    /** One line, shell-quoted. */
    text: string;
    /** Where hledger's figure can differ from the page's, and why. */
    caveat?: string;
}

const BILLABLE_CAVEAT =
    "hledger's tag:billable query matches the tag on any parent account, so an account marked billable: no under a billable one counts as both.";
const PERSON_CAVEAT = "hledger sees a person only where a person: tag is written; time filed under a person's folder without one isn't counted.";
const BILLED_CAVEAT = "The command leaves out the billing filter, which hledger can't say in one query.";
const SEARCH_CAVEAT = "The command leaves out the search, which looks in the notes and account names.";

/** The billing states hledger can say with a tag query. */
const BILLED_TAGS: Partial<Record<string, string>> = {invoiced: "tag:invoice", nobill: "tag:nobill"};

/** A shell word: as is when safe, else single-quoted. */
export function shellWord(word: string): string {
    return /^[A-Za-z0-9_./:=^-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`;
}

/** `-p 2026-11` for one whole month; else `-b FROM -e DAY-AFTER-TO` (hledger's end is exclusive). */
export function periodArgs(r: Pick<Range, "from" | "to">): string[] {
    if (isMonthStart(r.from) && isMonthEnd(r.to) && r.from.slice(0, 7) === r.to.slice(0, 7)) return ["-p", r.from.slice(0, 7)];
    return ["-b", r.from, "-e", addDays(r.to, 1)];
}

/** An account and everything below it, as an hledger account regex. */
export const accountRegex = (account: string): string => `^${account.replace(/[.*+?()[\]{}|\\$]/g, "\\$&")}(:|$)`;

/**
 * How a command uses accounts: `free` adds no pivot of its own (the billable
 * filter may then pivot), `pivot` pivots on another tag, `tree` needs the
 * account names (a tree or a depth).
 */
type Shape = "free" | "pivot" | "tree";

interface Built {
    args: string[];
    caveats: string[];
}

function filters(query: DashQuery, shape: Shape): Built {
    const args: string[] = [];
    const caveats: string[] = [];
    const account = query.account.trim();
    if (account !== "") args.push(accountRegex(account));
    const customer = query.customer.trim();
    if (customer !== "") args.push(`tag:customer=${customer}`);
    const person = query.person.trim();
    if (person !== "") {
        args.push(person === UNATTRIBUTED ? "not:tag:person" : `tag:person=${person}`);
        caveats.push(PERSON_CAVEAT);
    }
    if (query.billable !== "") {
        if (shape === "free") args.push("--pivot", "billable", query.billable === "yes" ? "^yes$" : "^no$");
        else {
            args.push(`tag:billable=${query.billable}`);
            caveats.push(BILLABLE_CAVEAT);
        }
    }
    if (query.billed !== "") {
        const tag = BILLED_TAGS[query.billed];
        if (tag === undefined) caveats.push(BILLED_CAVEAT);
        else args.push(tag);
    }
    if (query.q.trim() !== "") caveats.push(SEARCH_CAVEAT);
    return {args, caveats};
}

function command(
    file: string,
    r: Pick<Range, "from" | "to">,
    query: DashQuery,
    shape: Shape,
    report: string[],
    extraCaveats: string[] = [],
    verb: "bal" | "print" = "bal"
): Command {
    const f = filters(query, shape);
    const words = ["hledger", "-f", file, verb, ...periodArgs(r), ...f.args, ...report];
    const caveats = [...new Set([...f.caveats, ...extraCaveats])];
    return {text: words.map(shellWord).join(" "), ...(caveats.length === 0 ? {} : {caveat: caveats.join(" ")})};
}

/** The total: `bal --depth 0`. */
export const totalCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command => command(file, r, query, "free", ["--depth", "0"]);

/** Billable against not: `--pivot billable`. */
export function billableCommand(file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command {
    // The billable filter IS this pivot: keep its query, drop the second pivot.
    const report = query.billable === "" ? ["--pivot", "billable"] : [];
    return command(file, r, query, "free", report);
}

/** Per day, for the average and the days tracked: `-D --depth 0`. */
export const dailyCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command => command(file, r, query, "free", ["-D", "--depth", "0"]);

/** Billable time with a customer, not on an invoice and not written off. */
export function unbilledCommand(file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command {
    const q = {...query, billable: "yes" as const};
    return command(file, r, q, "free", ["tag:customer", "not:tag:invoice", "not:tag:nobill"]);
}

const GRAIN_FLAG: Record<Grain, string> = {day: "-D", week: "-W", month: "-M"};

const SPLIT_REPORT: Record<Split, {shape: Shape; args: string[]}> = {
    branch: {shape: "tree", args: ["--depth", "1"]},
    customer: {shape: "pivot", args: ["--pivot", "customer"]},
    person: {shape: "pivot", args: ["--pivot", "person"]},
    billable: {shape: "pivot", args: ["--pivot", "billable"]},
};

/** Hours over time: one column per bucket, one row per series. */
export function chartCommand(file: string, r: Pick<Range, "from" | "to">, query: DashQuery, grain: Grain, split: Split): Command {
    const {shape, args} = SPLIT_REPORT[split];
    if (split === "billable" && query.billable !== "") return command(file, r, query, "free", [GRAIN_FLAG[grain]]);
    return command(file, r, query, shape, [GRAIN_FLAG[grain], ...args], split === "person" ? [PERSON_CAVEAT] : []);
}

/** The account tree to `depth`. */
export const treeCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery, depth: number): Command =>
    command(file, r, query, "tree", ["--tree", "--depth", String(depth)]);

/** Accounts at `depth`, most hours first. */
export const topAccountsCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery, depth: number): Command =>
    command(file, r, query, "tree", ["--depth", String(depth), "-S"]);

/** Customers, most hours first. */
export const topCustomersCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command =>
    command(file, r, query, "pivot", ["--pivot", "customer", "-S"]);

/** People, most hours first. */
export const topPeopleCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command =>
    command(file, r, query, "pivot", ["--pivot", "person", "-S"], [PERSON_CAVEAT]);

/** hledger can't group by a note; `print` lists every entry with its note. */
export const notesCommand = (file: string, r: Pick<Range, "from" | "to">, query: DashQuery): Command =>
    command(file, r, query, "tree", [], ["hledger can't group by note text; print lists each entry with its note."], "print");
