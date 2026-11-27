// What the time dashboard needs beyond the period's figures, asked once per
// visit (and again after a file changes): whether time tracking is set up
// and the weekly target (`GET /api/time/state`), the account tree
// (`GET /api/time/accounts`: the customers to filter by, whether any
// billable account has one, the depth the tree goes to and a stable colour
// order) and everyone's all-time hours (`GET /api/time/summary?group=person`:
// the people to filter by, and whether there is any time at all).
//
// The decoders read only the fields used here, so this page doesn't break
// when Track or Accounts grow theirs.
import {send} from "#lib/editorApi.js";
import {fetchSummary} from "../api/summary.js";

export interface DashMeta {
    configured: boolean;
    /** Who is tracking on this machine, when set. */
    me: string | null;
    /** Hours a week; 0 is none. */
    weeklyTarget: number;
    /** Every customer a time account names, A–Z. */
    customers: string[];
    /** Everyone with time (`""`: unattributed), most hours first. */
    people: string[];
    /** Every time account, in file order (the account filter's suggestions). */
    accounts: string[];
    /** The deepest account's segment count. */
    maxDepth: number;
    /** Some billable account has a customer: the Unbilled tile applies. */
    billableCustomers: boolean;
    /** Any time at all, in any period. */
    anyTime: boolean;
    /** Stable colour orders: top-level branches and customers, most hours (all time) first. */
    branchOrder: string[];
    customerOrder: string[];
}

interface StateBody {
    configured: boolean;
    person: string | null;
    weekly_target: number;
}

interface AccountBody {
    account: string;
    depth: number;
    customer: {value: string; from: string} | null;
    billable: {value: string} | null;
    total: {seconds: number};
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

function isState(body: unknown): body is StateBody {
    return (
        isRecord(body) &&
        typeof body.configured === "boolean" &&
        (body.person === null || typeof body.person === "string") &&
        typeof body.weekly_target === "number"
    );
}

function isAccount(v: unknown): v is AccountBody {
    return (
        isRecord(v) &&
        typeof v.account === "string" &&
        typeof v.depth === "number" &&
        (v.customer === null || (isRecord(v.customer) && typeof v.customer.value === "string" && typeof v.customer.from === "string")) &&
        (v.billable === null || (isRecord(v.billable) && typeof v.billable.value === "string")) &&
        isRecord(v.total) &&
        typeof v.total.seconds === "number"
    );
}

function isAccounts(body: unknown): body is {accounts: AccountBody[]; undeclared: string[]} {
    return (
        isRecord(body) &&
        Array.isArray(body.accounts) &&
        body.accounts.every(isAccount) &&
        Array.isArray(body.undeclared) &&
        body.undeclared.every((u) => typeof u === "string")
    );
}

const byHours = (totals: ReadonlyMap<string, number>): string[] =>
    [...totals.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([k]) => k);

/** The meta from the three answers; pure, for tests. */
export function metaOf(
    state: StateBody,
    accounts: {accounts: AccountBody[]; undeclared: string[]},
    people: {key: string[]; quantity: {seconds: number}}[]
): DashMeta {
    const branches = new Map<string, number>();
    const customers = new Map<string, number>();
    let maxDepth = 1;
    let billableCustomers = false;
    for (const a of accounts.accounts) {
        maxDepth = Math.max(maxDepth, a.depth + 1);
        if (a.depth === 0) branches.set(a.account, a.total.seconds);
        if (a.customer !== null) {
            // The account that names the customer carries its whole subtree's time.
            const sum = customers.get(a.customer.value) ?? 0;
            customers.set(a.customer.value, a.customer.from === a.account ? sum + a.total.seconds : sum);
            if (a.billable?.value === "yes") billableCustomers = true;
        }
    }
    for (const u of accounts.undeclared) maxDepth = Math.max(maxDepth, u.split(":").length);
    const everyone = new Map(people.map((p) => [p.key[0] ?? "", p.quantity.seconds]));
    return {
        configured: state.configured,
        me: state.person,
        weeklyTarget: state.weekly_target,
        customers: [...customers.keys()].sort((a, b) => a.localeCompare(b)),
        people: byHours(everyone),
        // Undeclared accounts in use are rows of the tree too.
        accounts: [...new Set([...accounts.accounts.map((a) => a.account), ...accounts.undeclared])],
        maxDepth,
        billableCustomers,
        anyTime: people.some((p) => p.quantity.seconds !== 0),
        branchOrder: byHours(branches),
        customerOrder: byHours(customers),
    };
}

/** The meta, or `{configured: false}` before setup (when the other two routes refuse). */
export async function fetchDashMeta(): Promise<DashMeta> {
    const state = await send<StateBody>("GET", "/api/time/state", isState);
    if (!state.configured) {
        return metaOf(state, {accounts: [], undeclared: []}, []);
    }
    const [accounts, people] = await Promise.all([
        send<{accounts: AccountBody[]; undeclared: string[]}>("GET", "/api/time/accounts", isAccounts),
        fetchSummary({group: ["person"]}),
    ]);
    return metaOf(state, accounts, people.rows);
}
