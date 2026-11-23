// Ledgeline Invoicer's SPA. The same build as web/vite.config.ts (see the long
// comments there), with two differences: the app is served under `/app`, since
// MBR's router owns the root of the one server (invoicer/ARCHITECTURE.md §6.1),
// and the test suite is a single node project until there are components to
// mount.
import adapter from "@sveltejs/adapter-static";
import {vitePreprocess} from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import {sveltekit} from "@sveltejs/kit/vite";
import {defineConfig} from "vitest/config";

export default defineConfig({
    plugins: [
        tailwindcss(),
        sveltekit({
            preprocess: vitePreprocess(),
            // Runes for our code (and packages/ui's), never for node_modules libraries.
            dynamicCompileOptions({filename}) {
                return filename.split(/[/\\]/).includes("node_modules") ? undefined : {runes: true};
            },
            adapter: adapter({fallback: "index.html"}),
            paths: {base: "/app"},
            version: {pollInterval: 0},
        }),
    ],
    // The invoicer's ports differ from Ledgeline's (5173/4173 → 5099), so
    // both e2e suites can run side by side (invoicer PLAN 3a): this SPA on
    // 4273, the `ledgeline-invoicer --server` on 5199. The proxy keeps the
    // SPA's requests same-origin, as the embedded app's are; open
    // http://localhost:4273/app/#token=<the token --server prints>.
    server: {port: 4273, strictPort: true, proxy: {"/api": {target: "http://127.0.0.1:5199", changeOrigin: true}}},
    preview: {port: 4273, strictPort: true, proxy: {"/api": {target: "http://127.0.0.1:5199", changeOrigin: true}}},
    test: {
        expect: {requireAssertions: true},
        env: {TZ: "America/Denver"},
        environment: "node",
        include: ["src/**/*.{test,spec}.{js,ts}"],
    },
});
