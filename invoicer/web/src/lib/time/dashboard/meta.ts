// What the time dashboard needs beyond the period's figures, asked once per
// visit (and again after a file changes): whether time tracking is set up
// and the weekly target (`GET /api/time/state`), the account tree
// (`GET /api/time/accounts`: the customers to filter by, whether any
// billable account has one, the depth the tree goes to and a stable colour
// order) and everyone's all-time hours (`GET /api/time/summary?group=person`:
// the people to filter by, and whether there is any time at all).
//
// Each endpoint is read through its own module (../api/); `metaOf` takes
// only the fields it uses, so its tests stay small.
import type {AccountRow} from "../api/accounts.js";
import {fetchAccounts} from "../api/accounts.js";
import {fetchSummary} from "../api/summary.js";
import {fetchTrackState} from "../api/track.js";
import type {TrackState} from "../types.js";

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

type StateBody = Pick<TrackState, "configured" | "person" | "weekly_target">;

/** The fields of an account row the meta reads. */
type AccountBody = Pick<AccountRow, "account" | "depth"> & {
    customer: {value: string; from: string} | null;
    billable: {value: string} | null;
    total: {seconds: number};
};

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
    const state = await fetchTrackState();
    if (!state.configured) {
        return metaOf(state, {accounts: [], undeclared: []}, []);
    }
    const [accounts, people] = await Promise.all([fetchAccounts(), fetchSummary({group: ["person"]})]);
    return metaOf(state, accounts, people.rows);
}
