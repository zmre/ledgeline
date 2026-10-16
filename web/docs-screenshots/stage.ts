// Stages a scratch copy of one fixture set for a docs-screenshot engine.
//
//     bun docs-screenshots/stage.ts <set>
//
// Every engine the docs harness launches runs over a COPY under
// `target/docs-screenshots/<set>/`, never over `fixtures/` directly, for two
// reasons:
//
// 1. Nothing a shot does can rewrite a committed fixture. The e2e suite asserts
//    exact numbers from those files; a stray Save here must not move them.
// 2. The engine's rules-discovery scan root is the journal's directory. Over
//    `fixtures/` that lists the whole test corpus (edge cases, golden files,
//    scratch dirs), which is not what a reader's ledger looks like. The copy
//    holds one journal plus a small, realistic `imports/` folder.
//
// `target/` is gitignored and is not a discovery root for any e2e engine, so a
// leftover copy cannot leak into the e2e suite the way a dir under `fixtures/`
// would.
import {appendFileSync, copyFileSync, mkdirSync, rmSync} from "node:fs";
import {dirname, join, resolve} from "node:path";

const REPO = resolve(import.meta.dirname, "../..");
const FIXTURES = join(REPO, "fixtures");

/** `[source under fixtures/, destination under the set's dir]` pairs. */
type FixtureSet = Readonly<{files: ReadonlyArray<readonly [string, string]>; append?: Readonly<Record<string, string>>}>;

// Directives appended to the COPY only. They rename account names no posting
// uses, so no balance, report or check in the copy differs from the fixture —
// they exist so Settings → Account Aliases has rows to show.
const DOCS_ALIASES = `
; Added by web/docs-screenshots/stage.ts for the Account Aliases screenshot.
; None of these names appear in a posting, so no total changes.
alias Chase Checking 1234 = assets:bank:checking
alias Chase Sapphire Visa = liabilities:cc:visa
alias /^Vanguard Brokerage/ = assets:broker:taxable
`;

const SETS: Readonly<Record<string, FixtureSet>> = {
    sample: {
        files: [
            ["sample.journal", "sample.journal"],
            // Projections lists journals beside the main one; this is its asset-row scenario.
            ["asset-growth-scenario.journal", "asset-growth-scenario.journal"],
            ["rules/simple/checking.csv.rules", "imports/checking.csv.rules"],
            ["rules/simple/checking.csv", "imports/checking.csv"],
            ["rules/simple/creditcard1.csv.rules", "imports/creditcard1.csv.rules"],
            ["rules/simple/creditcard1.csv", "imports/creditcard1.csv"],
        ],
        append: {"sample.journal": DOCS_ALIASES},
    },
    budget: {files: [["budget/basic.journal", "basic.journal"]]},
    subscriptions: {files: [["subscriptions/basic.journal", "basic.journal"]]},
};

const stage = (name: string): string => {
    const set = SETS[name];
    if (!set) throw new Error(`unknown fixture set "${name}" (known: ${Object.keys(SETS).join(", ")})`);
    const dest = join(REPO, "target", "docs-screenshots", name);
    rmSync(dest, {recursive: true, force: true});
    for (const [from, to] of set.files) {
        const target = join(dest, to);
        mkdirSync(dirname(target), {recursive: true});
        copyFileSync(join(FIXTURES, from), target);
    }
    for (const [file, text] of Object.entries(set.append ?? {})) appendFileSync(join(dest, file), text);
    return dest;
};

const name = process.argv[2];
if (!name) throw new Error("usage: bun docs-screenshots/stage.ts <set>");
console.log(`staged ${name} → ${stage(name)}`);
