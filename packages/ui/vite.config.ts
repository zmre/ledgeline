// Test config only: this package has no build of its own. Each app's vite build
// compiles its sources (it is linked into node_modules as source), so the
// svelte options here must match the apps': runes everywhere but node_modules.
//
// The suite is split the way web/'s is, and for the same reasons (see the long
// comments in web/vite.config.ts): `unit` runs in node and needs nothing,
// `components` mounts `*.svelte.test.ts` in jsdom with the browser build of
// Svelte.
import {svelte, vitePreprocess} from "@sveltejs/vite-plugin-svelte";
import {defineConfig} from "vitest/config";

export default defineConfig({
    plugins: [
        svelte({
            preprocess: vitePreprocess(),
            dynamicCompileOptions({filename}) {
                return filename.split(/[/\\]/).includes("node_modules") ? undefined : {runes: true};
            },
        }),
    ],
    test: {
        // Inherited by both projects through `extends`, as in web/vite.config.ts.
        expect: {requireAssertions: true},
        // The developer's zone, which CI pins too (see web/vite.config.ts).
        env: {TZ: "America/Denver"},
        projects: [
            {
                extends: "./vite.config.ts",
                test: {
                    name: "unit",
                    environment: "node",
                    include: ["src/**/*.{test,spec}.{js,ts}"],
                    exclude: ["src/**/*.svelte.{test,spec}.{js,ts}"],
                },
            },
            {
                extends: "./vite.config.ts",
                resolve: {conditions: ["browser"]},
                test: {
                    name: "components",
                    environment: "jsdom",
                    include: ["src/**/*.svelte.{test,spec}.{js,ts}"],
                    setupFiles: ["./src/testing/componentSetup.ts"],
                },
            },
        ],
    },
});
