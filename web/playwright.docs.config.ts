import {defineConfig} from "@playwright/test";

// Docs screenshot harness (plans/25-published-docs.md, "Screenshots").
//
//     just docs-screenshots
//
// NOT part of the e2e suite: its specs live in docs-screenshots/ and end in
// `.docs.ts`, which neither `testDir: "e2e"` nor `*.e2e.ts` in
// playwright.config.ts can match. Its engines use their own ports (521x), so it
// can run while an e2e run is up.
//
// Each fixture set gets its own engine over a scratch copy staged by
// docs-screenshots/stage.ts (see there for why never over fixtures/ directly).
//
// SAME-ORIGIN, unlike the e2e suite: the browser loads the SPA the engine
// itself embeds (crates/ledgeline-server/build.rs mirrors web/build at
// `cargo build` time, which is why `just docs-screenshots` builds the SPA
// first), exactly as the packaged app does. Screenshots should show that app,
// and it sidesteps CORS: the cross-origin preflight does not allow the
// `X-Ledgeline-Filename` header a file drop sends, so a QuickBooks drop from
// a separate preview origin fails. The engine a page talks to is chosen per
// spec by the `journal` fixture option in docs-screenshots/harness.ts.

/** Access token every docs engine is launched with. */
export const DOCS_TOKEN = "ledgeline-docs-token";

/** Fixture set → engine port. The keys are stage.ts set names. */
export const DOCS_ENGINES = {sample: 5211, budget: 5212, subscriptions: 5213} as const;
export type DocsJournal = keyof typeof DOCS_ENGINES;

/** The journal file each staged set's engine opens. */
const JOURNAL_FILE: Readonly<Record<DocsJournal, string>> = {
    sample: "sample.journal",
    budget: "basic.journal",
    subscriptions: "basic.journal",
};

const BIN = process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline";

const engine = (set: DocsJournal) => ({
    command: `bun docs-screenshots/stage.ts ${set} && exec ${BIN} --server ../target/docs-screenshots/${set}/${JOURNAL_FILE[set]} --port ${DOCS_ENGINES[set]}`,
    env: {LEDGELINE_TOKEN: DOCS_TOKEN},
    url: `http://127.0.0.1:${DOCS_ENGINES[set]}/`,
    reuseExistingServer: false,
});

export default defineConfig({
    webServer: (Object.keys(DOCS_ENGINES) as DocsJournal[]).map(engine),
    use: {
        timezoneId: "America/Denver",
        locale: "en-US",
        viewport: {width: 1280, height: 800},
        deviceScaleFactor: 2,
        colorScheme: "dark",
    },
    testDir: "docs-screenshots",
    testMatch: "**/*.docs.ts",
    // Screenshots are the product, not assertions: a retry hides nothing useful,
    // and a flake should be looked at.
    retries: 0,
    reporter: [["list"]],
});
