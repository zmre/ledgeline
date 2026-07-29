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
            env: {LEDGELINE_TOKEN: API_TOKEN},
            // The SPA shell, not /version: every wire route now needs the token.
            url: "http://127.0.0.1:5099/",
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
