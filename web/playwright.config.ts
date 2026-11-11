import {defineConfig} from "@playwright/test";

/** Access token the engine is launched with, and that every spec seeds. */
export const API_TOKEN = "ledgeline-e2e-token";

export default defineConfig({
    webServer: [
        {
            // Fixture API for the e2e suite. The `ledgeline` binary in --server
            // (headless) mode serves BOTH the wire endpoints (journal view +
            // checks) and the native /api/* report/holdings endpoints the
            // reports/holdings pages now consume.
            //
            // The suite runs the built SPA at :4173 and the engine at :5099 —
            // CROSS-ORIGIN — so it needs both halves of the SEC-1 opt-in:
            //   --allow-origin http://localhost:4173  (exact origin; never '*')
            //   LEDGELINE_TOKEN=...                   (deterministic, so the
            //     specs can seed it into localStorage alongside serverUrl)
            // The packaged app needs neither: it serves the SPA same-origin.
            // Build it first: `cargo build -p ledgeline-server` (see the repo
            // README/plans); the compiled binary lives at ../target/debug/ledgeline.
            // $LEDGELINE_BIN overrides that path: CI has no cargo target dir, it
            // runs the Nix-built binary (`nix build .#ledgeline` → result/bin/ledgeline).
            command: `${process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline"} --server ../fixtures/sample.journal --port 5099 --allow-origin http://localhost:4173`,
            // LEDGELINE_IGNORE_USER_HLEDGER_CONF: never read the developer's own
            // ~/.hledger.conf (Settings › hledger config would show it).
            env: {LEDGELINE_TOKEN: API_TOKEN, LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1"},
            // The SPA shell, not /version: every wire route now needs the token.
            url: "http://127.0.0.1:5099/",
            reuseExistingServer: false,
        },
        {
            // The transaction-review engine (plan 32, e2e/review.e2e.ts), over
            // fixtures/review/ so sample.journal's totals never move. Same
            // cross-origin opt-in as the engine above.
            //
            // It runs over a SCRATCH COPY (target/e2e-review/, gitignored): the
            // review spec saves, and a save must never rewrite a committed
            // fixture. Re-staged on every run, so each run starts from the
            // fixture's bytes. `review-2025.journal` is the main file's include.
            command: `rm -rf ../target/e2e-review && mkdir -p ../target/e2e-review && cp ../fixtures/review/review.journal ../fixtures/review/review-2025.journal ../target/e2e-review/ && exec ${process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline"} --server ../target/e2e-review/review.journal --port 5098 --allow-origin http://localhost:4173`,
            env: {LEDGELINE_TOKEN: API_TOKEN, LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1"},
            url: "http://127.0.0.1:5098/",
            reuseExistingServer: false,
        },
        {
            // The import-review engine (plan 32 Phase 4, the "review before
            // writing" test in e2e/imports.e2e.ts). That test WRITES twice — a
            // rule into the rules file, then the import into the journal — so,
            // like the review engine, it runs over a scratch copy staged fresh
            // on every run (target/e2e-import-review/, gitignored): an empty
            // journal and `imports/checking.csv.rules`, the docs' checking
            // account rules.
            command: `rm -rf ../target/e2e-import-review && mkdir -p ../target/e2e-import-review/imports && cp ../fixtures/rules/simple/checking.csv.rules ../target/e2e-import-review/imports/ && echo "; Staged by web/playwright.config.ts for the import-review e2e. Not a fixture." > ../target/e2e-import-review/main.journal && exec ${process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline"} --server ../target/e2e-import-review/main.journal --port 5097 --allow-origin http://localhost:4173`,
            env: {LEDGELINE_TOKEN: API_TOKEN, LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1"},
            url: "http://127.0.0.1:5097/",
            reuseExistingServer: false,
        },
        {
            // The journal-rules engine (plan 32 D27, "Remember as a rule from
            // the Journal" in e2e/review.e2e.ts), over fixtures/review-rules/:
            // the only fixture whose journal's unknowns came from rules files
            // beside it. fixtures/review/ has none, so the review engine above
            // never offers the toggle. The test saves a rule and a card, so
            // this is a scratch copy too (target/e2e-review-rules/, gitignored),
            // staged fresh on every run. `cp -R` keeps the `.latest.*` markers.
            command: `rm -rf ../target/e2e-review-rules && mkdir -p ../target && cp -R ../fixtures/review-rules ../target/e2e-review-rules && exec ${process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline"} --server ../target/e2e-review-rules/main.journal --port 5096 --allow-origin http://localhost:4173`,
            env: {LEDGELINE_TOKEN: API_TOKEN, LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1"},
            url: "http://127.0.0.1:5096/",
            reuseExistingServer: false,
        },
        // MERGE SEAM: plan 33's engine (:5095). Another plan's scratch engine
        // goes beside it, on its own port.
        {
            // The default-projection engine (plan 33, e2e/projectionDefault.e2e.ts),
            // over a SCRATCH COPY of fixtures/projection-default/
            // (target/e2e-projection-default/, gitignored), staged fresh on every
            // run: the spec stars, un-stars and saves scenario files, and a write
            // must never touch a committed fixture. The two scenarios are
            // committed as `*.scenario` and renamed to `projection-*.journal` in
            // the copy: under their real names the 5099 engine's scan of
            // fixtures/ would list them beside sample.journal.
            command: `rm -rf ../target/e2e-projection-default && mkdir -p ../target && cp -R ../fixtures/projection-default ../target/e2e-projection-default && mv ../target/e2e-projection-default/alpha.scenario ../target/e2e-projection-default/projection-alpha.journal && mv ../target/e2e-projection-default/beta.scenario ../target/e2e-projection-default/projection-beta.journal && exec ${process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline"} --server ../target/e2e-projection-default/main.journal --port 5095 --allow-origin http://localhost:4173`,
            env: {LEDGELINE_TOKEN: API_TOKEN, LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1"},
            url: "http://127.0.0.1:5095/",
            reuseExistingServer: false,
        },
        {
            // The startup-metrics engine (plans/37, e2e/startupMetrics.e2e.ts),
            // over fixtures/saas/startup.journal: a small SaaS business whose
            // expected figures are listed in fixtures/saas/README.md. Served in
            // place, read-only: the spec only reads (and exports) and never
            // saves, so there is no scratch copy to stage.
            command: `${process.env.LEDGELINE_BIN ?? "../target/debug/ledgeline"} --server ../fixtures/saas/startup.journal --port 5094 --allow-origin http://localhost:4173`,
            env: {LEDGELINE_TOKEN: API_TOKEN, LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1"},
            url: "http://127.0.0.1:5094/",
            reuseExistingServer: false,
        },
        {command: "bun run build && bun run preview", port: 4173},
    ],
    // With multiple webServers playwright does not infer baseURL from `port`.
    // timezoneId matches the vitest pin in vite.config.ts so both suites exercise
    // a negative-offset zone rather than whatever the runner happens to be in.
    use: {baseURL: "http://localhost:4173", timezoneId: "America/Denver"},
    testDir: "e2e",
    testMatch: "**/*.e2e.{ts,js}",
});
