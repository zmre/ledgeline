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
    test: {
        expect: {requireAssertions: true},
        env: {TZ: "America/Denver"},
        environment: "node",
        include: ["src/**/*.{test,spec}.{js,ts}"],
    },
});
