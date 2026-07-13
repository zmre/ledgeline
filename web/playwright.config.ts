import {defineConfig} from "@playwright/test";

export default defineConfig({
    webServer: [
        {
            // Fixture API for the e2e smoke suite (WP-09). --serve-api never idle-exits.
            command: "hledger-web -f ../fixtures/sample.journal --serve-api --cors='*' --allow=view --port 5099",
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
