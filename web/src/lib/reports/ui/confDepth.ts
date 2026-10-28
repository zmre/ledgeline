// What `hledger.conf`'s `--depth` means on the Reports page (plan 30,
// Decision 10). Pure.
//
// Cash Flow and Net Worth have a depth control, and the conf's plain depth is
// its default (see `params.ts::confDepthDefault`). The Balance Sheet and P&L
// deliberately have none (plans 12/13); there the depth decides which groups
// START expanded, per account when `REGEX=N` says so.
//
// The per-account rule mirrors hledger 1.52, verified with scratch journals:
// a REGEX is matched case-insensitively anywhere in the full account name; the
// plain depth limits every account no REGEX matches; and when several match,
// the one whose match sits furthest right in the name wins (`broker=3` beats
// `inv=4` for `assets:inv:broker:fund`, either order), ties going to the one
// declared last.

import type {DepthSpec} from "#lib/settings/hledgerConfTypes.js";

/** Whether a spec says anything at all. An empty one means "no depth configured", not "unlimited". */
export function isConfigured(spec: DepthSpec | null | undefined): spec is DepthSpec {
    return spec !== null && spec !== undefined && (spec.all !== null || spec.perRegex.length > 0);
}

/** The depth hledger shows `account` at under `spec`: Infinity when nothing limits it. */
export function depthLimit(spec: DepthSpec, account: string): number {
    let best: {at: number; depth: number} | null = null;
    for (const {regex, depth} of spec.perRegex) {
        let at: number;
        try {
            at = account.search(new RegExp(regex, "i"));
        } catch {
            continue; // hledger would refuse the file; the settings screen warns about it
        }
        if (at >= 0 && (best === null || at >= best.at)) best = {at, depth};
    }
    return best?.depth ?? spec.all ?? Infinity;
}

/** `account` cut to its first `depth` components. */
function clipped(account: string, depth: number): string {
    return Number.isFinite(depth) ? account.split(":").slice(0, Math.max(depth, 1)).join(":") : account;
}

/**
 * Whether a group of the grouped statements starts expanded: when hledger, at
 * this depth, would show its accounts as MORE THAN ONE line. A group whose
 * accounts all roll up into one name reads exactly as its own heading does.
 * With no depth configured the answer is the screens' long-standing default:
 * every group starts collapsed.
 */
export function groupStartsOpen(accounts: readonly string[], spec: DepthSpec | null | undefined): boolean {
    if (!isConfigured(spec)) return false;
    return new Set(accounts.map((account) => clipped(account, depthLimit(spec, account)))).size > 1;
}

/** One group of a grouped statement, as `groupsToOpen` needs it: its collapse key and its member accounts. */
export interface DepthGroup {
    readonly key: string;
    readonly accounts: readonly string[];
}

/** The keys of the groups `spec` starts expanded. */
export function groupsToOpen(groups: readonly DepthGroup[], spec: DepthSpec | null | undefined): string[] {
    return groups.filter((group) => groupStartsOpen(group.accounts, spec)).map((group) => group.key);
}

/**
 * The seeding state a grouped statement keeps: which group keys a decision
 * has already been made for, under which spec. Plain data — a view holds one
 * in a plain `let`, never in `$state`, so the effect that seeds cannot depend
 * on its own writes.
 */
export interface OpenSeed {
    spec: string;
    decided: Set<string>;
}

export const newOpenSeed = (): OpenSeed => ({spec: "", decided: new Set()});

/**
 * Which of `groups` to open NOW: the ones `spec` opens that have not been
 * decided before. A group is decided once — so collapsing one the config
 * opened sticks across refetches — until the spec itself changes (a config
 * edit), when every group is decided afresh. Opening only ever adds: nothing
 * the user opened is shut.
 */
export function seedOpenGroups(seed: OpenSeed, groups: readonly DepthGroup[], spec: DepthSpec | null | undefined): string[] {
    const specKey = JSON.stringify(spec ?? null);
    if (specKey !== seed.spec) {
        seed.spec = specKey;
        seed.decided.clear();
    }
    const fresh = groups.filter((group) => !seed.decided.has(group.key));
    for (const group of fresh) seed.decided.add(group.key);
    return groupsToOpen(fresh, spec);
}
