// DECODED alias values for the component tests — deliberately not wire JSON.
//
// `importFixtures.ts` holds the literal bytes the engine sends, because the
// tests there are about what the store and the decoders make of a CONTRACT.
// These are the other side of that line: `AliasEffectPanel` and `DryRunPanel`
// take decoded props and touch no store at all, so the honest input to a mount
// of one is a typed value, not a JSON blob that has to be pushed through a
// fetch stub and a decoder first to arrive at the same object.
//
// The account names are long on purpose. `PW Roth IRA - 3077:cash` →
// `assets:morganstanley:pw-roth-ira:cash` is the real pair from the report that
// prompted this panel's rewrite, and a fixture of `a → b` would render happily
// in any layout, including the broken one.

import type {AliasEffect, AliasEntry, DryRunResult} from "$lib/imports/importTypes";

/** The plain alias in the user's journal that does the rewriting below. */
export const ROTH_ALIAS: AliasEntry = {
    journalId: "2026/2026.journal",
    index: 0,
    line: 12,
    pattern: "PW Roth IRA - 3077",
    replacement: "assets:morganstanley:pw-roth-ira",
    regex: false,
    forwarded: true,
    refusal: null,
    refusalMessage: null,
    editable: true,
    lock: null,
    lockMessage: null,
};

/** Aliases rewrite two accounts, and a terminal would agree — the ordinary case. */
export const RENAMES_ONLY: AliasEffect = {
    forwarded: 1,
    renames: [
        {from: "PW Roth IRA - 3077:cash", to: "assets:morganstanley:pw-roth-ira:cash"},
        {from: "PW Roth IRA - 3077", to: "assets:morganstanley:pw-roth-ira"},
    ],
    cli: {
        matches: true,
        differences: [],
        confPath: "hledger.conf",
        confOutside: false,
        confHijackedBy: null,
        additions: [],
        refusals: [],
        revision: "rev-conf-1",
        writable: true,
    },
};

/**
 * The same rewrite, plus the measured divergence from a command-line import —
 * with NO config file in force, which is the overwhelmingly common shape of it.
 *
 * Note what `differences` is: exactly one of the renames above, character for
 * character. That is not fixture laziness, it is the situation. With no
 * `hledger.conf`, a terminal writes the account the rules file produced and
 * Ledgeline writes the aliased one, so the divergence list IS the rename list.
 * Printing both was the "two alert boxes … both saying much the same thing"
 * report, and `parityRepeatsRenames` is what now notices.
 */
export const RENAMES_AND_PARITY: AliasEffect = {
    ...RENAMES_ONLY,
    cli: {
        matches: false,
        // Command-line answer → Ledgeline's.
        differences: [{from: "PW Roth IRA - 3077:cash", to: "assets:morganstanley:pw-roth-ira:cash"}],
        confPath: null,
        confOutside: false,
        confHijackedBy: null,
        additions: ["PW.Roth.IRA.-.3077=assets:morganstanley:pw-roth-ira"],
        refusals: [],
        revision: "",
        writable: true,
    },
};

/**
 * A divergence the rename list does NOT already show.
 *
 * An `hledger.conf` beside the journal supplies some of the mapping and not the
 * rest, so a terminal lands on an account no alias in the journal produces —
 * a pair that appears in neither the rename list nor anywhere else on screen,
 * and therefore has to be printed.
 */
export const PARITY_BEYOND_RENAMES: AliasEffect = {
    ...RENAMES_ONLY,
    cli: {
        matches: false,
        differences: [{from: "assets:morganstanley:roth", to: "assets:morganstanley:pw-roth-ira"}],
        confPath: "hledger.conf",
        confOutside: false,
        confHijackedBy: null,
        additions: ["PW.Roth.IRA.-.3077=assets:morganstanley:pw-roth-ira"],
        refusals: [],
        revision: "rev-conf-1",
        writable: true,
    },
};

/** Aliases are in force but matched nothing here, and the two tools agree: say nothing. */
export const ALIASES_QUIET: AliasEffect = {
    forwarded: 1,
    renames: [],
    cli: {...RENAMES_ONLY.cli},
};

/** A successful dry run, with `aliases` left to the caller. */
export function dryRun(aliases: AliasEffect | null): DryRunResult {
    return {
        ok: true,
        entries: "2026-06-24 Coffee\n    expenses:food  $4.50\n    assets:bank:checking\n",
        count: 1,
        status: "would import 1 new transaction from bank.csv:",
        skipped: null,
        balance: null,
        aliases,
        blockedByGit: [],
    };
}
