import {defineConfig} from "@playwright/test";

/**
 * The invoicer's e2e suite (invoicer PLAN 3b), set up as web/playwright.config.ts
 * is, on the invoicer's own ports (PLAN 3a): the server on 5199, this SPA's
 * `vite preview` on 4273. Ledgeline's suite uses 5092–5099 and 4173, so both
 * can run at once.
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
        {command: "bun run build && bun run preview", port: 4273},
    ],
    // timezoneId matches vitest's pin (vite.config.ts); dark, like Ledgeline's suite.
    use: {baseURL: "http://localhost:4273", timezoneId: "America/Denver", colorScheme: "dark"},
    testDir: "e2e",
    testMatch: "**/*.e2e.{ts,js}",
});
