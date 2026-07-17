import {defineConfig} from "@playwright/test";

export default defineConfig({
    webServer: [
        {
            // Fixture API for the e2e suite. ledgeline-server serves BOTH the wire
            // endpoints (journal view + checks) and the native /api/* report/holdings
            // endpoints the reports/holdings pages now consume, with permissive CORS
            // built in. Build it first: `cargo build -p ledgeline-server` (see the
            // repo README/plans); the compiled binary lives at ../target/debug/.
            command: "../target/debug/ledgeline-server ../fixtures/sample.journal --port 5099",
            url: "http://127.0.0.1:5099/version",
            reuseExistingServer: false,
        },
        {command: "bun run build && bun run preview", port: 4173},
    ],
    // With multiple webServers playwright does not infer baseURL from `port`.
    use: {baseURL: "http://localhost:4173"},
    testDir: "e2e",
    testMatch: "**/*.e2e.{ts,js}",
});
