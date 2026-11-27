import {defineConfig} from "@playwright/test";

/**
 * The invoicer's e2e suite (invoicer PLAN 3b), set up as web/playwright.config.ts
 * is, on the invoicer's own ports (PLAN 3a). Ledgeline's suite uses 5092–5099
 * and 4173, so both can run at once. The ports, each over its own scratch copy
 * of a fixture under target/:
 *   - 5199: the main server, the accrual books (most specs);
 *   - 5198: the empty books (empty.e2e.ts);
 *   - 5197: the cash books with an import appended (cash.e2e.ts);
 *   - 5196: the cash books for the Settings page (settings.e2e.ts);
 *   - 5195: books with no ledgeline.toml (settings-new-file.e2e.ts);
 *   - 5194: the time-tracking books (time-track.e2e.ts, plan 46);
 *   - 5193: the time books for Entries and Accounts (time-entries.e2e.ts);
 *   - 4273: this SPA's `vite preview`, proxying to 5199.
 *
 * Run it with `just invoicer-e2e` (it builds the SPA, then the debug binary,
 * which embeds that build, then runs this). $LEDGELINE_INVOICER_BIN overrides
 * the binary, as $LEDGELINE_BIN does for Ledgeline's.
 */

/** The access token the server is launched with, and that every spec hands the SPA. */
export const API_TOKEN = "ledgeline-invoicer-e2e-token";

/** The server, for specs that call the API or load the embedded SPA directly. */
export const SERVER_URL = "http://127.0.0.1:5199";

/**
 * The scratch copy of invoicer/fixtures/accrual the server runs over
 * (target/e2e-invoicer-accrual/, gitignored), relative to this folder. Staged
 * fresh on every run: the server's reconciler may write `paid:` mirrors, and
 * nothing may ever write a committed fixture.
 */
export const COPY = "../../target/e2e-invoicer-accrual";

/**
 * The second server (Phase 4 feedback H22): `--server` on 5198 over a scratch
 * copy of invoicer/fixtures/empty (no invoices, customers, catalog or
 * snippets), for empty.e2e.ts. `vite preview` proxies only to 5199, so that
 * spec loads the SPA this server embeds, as csp.same-origin.e2e.ts does.
 */
export const EMPTY_SERVER_URL = "http://127.0.0.1:5198";
export const EMPTY_COPY = "../../target/e2e-invoicer-empty";

/**
 * The third server: `--server` on 5197 over a scratch copy of
 * invoicer/fixtures/cash with one imported, unapplied deposit appended
 * (e2e/fixtures/cash-imports.journal), for cash.e2e.ts: Apply payment and New
 * deposit invoice, in books no other spec reads. Like EMPTY_SERVER_URL, its
 * spec loads the SPA the server embeds.
 */
export const CASH_SERVER_URL = "http://127.0.0.1:5197";
export const CASH_COPY = "../../target/e2e-invoicer-cash";

/**
 * The fourth server: `--server` on 5196 over a scratch copy of
 * invoicer/fixtures/cash, with one customer added to the copy
 * (e2e/fixtures/settings-adatum.md: no terms, no template), for
 * settings.e2e.ts. That spec saves the copy's ledgeline.toml and edits it
 * from outside, so it has books of its own. Like CASH_SERVER_URL, its spec
 * loads the SPA the server embeds.
 */
export const SETTINGS_SERVER_URL = "http://127.0.0.1:5196";
export const SETTINGS_COPY = "../../target/e2e-invoicer-settings";

/**
 * The fifth server: `--server` on 5195 over a scratch copy of
 * invoicer/fixtures/empty with its ledgeline.toml deleted from the copy, for
 * settings-new-file.e2e.ts: Save asks before it creates the settings file.
 */
export const NEW_FILE_SERVER_URL = "http://127.0.0.1:5195";
export const NEW_FILE_COPY = "../../target/e2e-invoicer-no-settings";

/**
 * The sixth server: `--server` on 5194 over a scratch copy of
 * invoicer/fixtures/time (plan 46), for time-track.e2e.ts: the tracker
 * writes the copy's time files on the real clock.
 */
export const TIME_SERVER_URL = "http://127.0.0.1:5194";
export const TIME_COPY = "../../target/e2e-invoicer-time";

/**
 * The seventh server (plan 46 Phase 4): `--server` on 5193 over a scratch copy
 * of invoicer/fixtures/time, for time-entries.e2e.ts, which adds, edits,
 * writes off and deletes time and renames an account. Like CASH_SERVER_URL,
 * its spec loads the SPA the server embeds.
 */
export const TIME_ENTRIES_SERVER_URL = "http://127.0.0.1:5193";
export const TIME_ENTRIES_COPY = "../../target/e2e-invoicer-time-entries";

const BIN = process.env.LEDGELINE_INVOICER_BIN ?? "../../target/debug/ledgeline-invoicer";

/**
 * The server's environment. `LEDGELINE_INVOICER_TOKEN` pins the token so the
 * specs know it. `LEDGELINE_UI_PREFS_IN_BROWSER` keeps theme and scale in the
 * browser (no write to the developer's ui-prefs.toml), and
 * `LEDGELINE_IGNORE_USER_HLEDGER_CONF` keeps the developer's ~/.hledger.conf
 * out, as in Ledgeline's suite.
 */
const SERVER_ENV = {
    LEDGELINE_INVOICER_TOKEN: API_TOKEN,
    LEDGELINE_UI_PREFS_IN_BROWSER: "1",
    LEDGELINE_IGNORE_USER_HLEDGER_CONF: "1",
};

export default defineConfig({
    webServer: [
        {
            // `--server` over the scratch copy. The SPA reaches it through
            // vite's proxy (vite.config.ts), so every request is same-origin,
            // as in the app: no --allow-origin is needed, and MBR's pages get
            // the session cookie.
            command: `rm -rf ${COPY} && mkdir -p ../../target && cp -R ../fixtures/accrual ${COPY} && exec ${BIN} --server ${COPY}/books.journal --port 5199`,
            env: SERVER_ENV,
            // The SPA shell answers without the token; every /api route needs it.
            url: `${SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {
            // The empty books (H22), staged fresh like the accrual copy.
            command: `rm -rf ${EMPTY_COPY} && mkdir -p ../../target && cp -R ../fixtures/empty ${EMPTY_COPY} && exec ${BIN} --server ${EMPTY_COPY}/books.journal --port 5198`,
            env: SERVER_ENV,
            url: `${EMPTY_SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {
            // The cash books, staged fresh with the import appended to the copy only.
            command: `rm -rf ${CASH_COPY} && mkdir -p ../../target && cp -R ../fixtures/cash ${CASH_COPY} && cat e2e/fixtures/cash-imports.journal >> ${CASH_COPY}/books.journal && exec ${BIN} --server ${CASH_COPY}/books.journal --port 5197`,
            env: SERVER_ENV,
            url: `${CASH_SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {
            // The cash books again, for the Settings page, with a customer that sets no terms.
            command: `rm -rf ${SETTINGS_COPY} && mkdir -p ../../target && cp -R ../fixtures/cash ${SETTINGS_COPY} && cp e2e/fixtures/settings-adatum.md ${SETTINGS_COPY}/customers/Adatum.md && exec ${BIN} --server ${SETTINGS_COPY}/books.journal --port 5196`,
            env: SERVER_ENV,
            url: `${SETTINGS_SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {
            // The empty books with no settings file: deleted from the copy, never the fixture.
            command: `rm -rf ${NEW_FILE_COPY} && mkdir -p ../../target && cp -R ../fixtures/empty ${NEW_FILE_COPY} && rm ${NEW_FILE_COPY}/ledgeline.toml && exec ${BIN} --server ${NEW_FILE_COPY}/books.journal --port 5195`,
            env: SERVER_ENV,
            url: `${NEW_FILE_SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {
            // The time-tracking books (plan 46), staged fresh like the others.
            command: `rm -rf ${TIME_COPY} && mkdir -p ../../target && cp -R ../fixtures/time ${TIME_COPY} && exec ${BIN} --server ${TIME_COPY}/books.journal --port 5194`,
            env: SERVER_ENV,
            url: `${TIME_SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {
            // The time books (plan 46), staged fresh like the others.
            command: `rm -rf ${TIME_ENTRIES_COPY} && mkdir -p ../../target && cp -R ../fixtures/time ${TIME_ENTRIES_COPY} && exec ${BIN} --server ${TIME_ENTRIES_COPY}/books.journal --port 5193`,
            env: SERVER_ENV,
            url: `${TIME_ENTRIES_SERVER_URL}/app/`,
            reuseExistingServer: false,
        },
        {command: "bun run build && bun run preview", port: 4273},
    ],
    // timezoneId matches vitest's pin (vite.config.ts); dark, like Ledgeline's suite.
    use: {baseURL: "http://localhost:4273", timezoneId: "America/Denver", colorScheme: "dark"},
    testDir: "e2e",
    testMatch: "**/*.e2e.{ts,js}",
    // Every spec shares one server and one scratch copy of the books, so the
    // specs that change it run in turn, each a project after the last:
    //   0. dashboard.e2e.ts (5a) only reads, but reads the fixture's figures
    //      as committed (the totals, aging, one draft), so it goes first;
    //   1. customers.e2e.ts reads the fixture's balances (Customer X owes
    //      $8,350.00, 108 a draft), creates Northwind Traders and removes it;
    //   2. the rest (invoices.e2e.ts sends 108, so Customer X owes more; it
    //      counts eight invoices);
    //   3. editor.e2e.ts adds invoice 2005 and expects 108 sent (and the
    //      catalog's ARCH as the fixture has it);
    //   4. library.e2e.ts edits the catalog and the snippets, and puts them
    //      back after;
    //   5. templates.e2e.ts (4d) creates a template, uses it on 101 and
    //      deletes it, putting 101 back;
    //   6. lists.e2e.ts (PLAN 5b) filters and sorts both lists over what the
    //      others leave (it relies only on facts they don't change, 101 paid
    //      among them, which templates.e2e.ts puts back first), and adds,
    //      edits and closes an open PO on Customer X (a close notes it in the
    //      body), then writes Customer X back byte for byte;
    //   7. keyboard.e2e.ts (PLAN 5k) drives the lists by keys, then writes,
    //      sends and exports an invoice for Fabrikam without the mouse; last,
    //      since every figure above would see it.
    // empty.e2e.ts has its own server and books (5198), so it runs alongside;
    // so do cash.e2e.ts (5197, the cash books), settings.e2e.ts (5196, its
    // own cash books), settings-new-file.e2e.ts (5195, no settings file),
    // time-track.e2e.ts (5194) and time-entries.e2e.ts (5193), the time books.
    projects: [
        {name: "dashboard", testMatch: "**/dashboard.e2e.ts"},
        {name: "customers", testMatch: "**/customers.e2e.ts", dependencies: ["dashboard"]},
        {name: "empty", testMatch: "**/empty.e2e.ts"},
        {name: "cash", testMatch: "**/cash.e2e.ts"},
        {name: "settings", testMatch: "**/settings.e2e.ts"},
        {name: "settings-new-file", testMatch: "**/settings-new-file.e2e.ts"},
        {name: "time", testMatch: "**/time-track.e2e.ts"},
        // Same server as Track; after it, so the two never write at once.
        {name: "time-dashboard", testMatch: "**/time-dashboard.e2e.ts", dependencies: ["time"]},
        {name: "time-entries", testMatch: "**/time-entries.e2e.ts"},
        {
            name: "pages",
            testIgnore: [
                "**/dashboard.e2e.ts",
                "**/customers.e2e.ts",
                "**/editor.e2e.ts",
                "**/library.e2e.ts",
                "**/empty.e2e.ts",
                "**/cash.e2e.ts",
                "**/settings.e2e.ts",
                "**/settings-new-file.e2e.ts",
                "**/time-track.e2e.ts",
                "**/time-dashboard.e2e.ts",
                "**/time-entries.e2e.ts",
                "**/templates.e2e.ts",
                "**/lists.e2e.ts",
                "**/keyboard.e2e.ts",
            ],
            dependencies: ["customers"],
        },
        {name: "editor", testMatch: "**/editor.e2e.ts", dependencies: ["pages"]},
        {name: "library", testMatch: "**/library.e2e.ts", dependencies: ["editor"]},
        {name: "templates", testMatch: "**/templates.e2e.ts", dependencies: ["library"]},
        {name: "lists", testMatch: "**/lists.e2e.ts", dependencies: ["templates"]},
        {name: "keyboard", testMatch: "**/keyboard.e2e.ts", dependencies: ["lists"]},
    ],
});
