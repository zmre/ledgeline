// Ledgeline Invoicer's SPA. The same build as web/vite.config.ts (see the long
// comments there), with one difference: the app is served under `/app`, since
// MBR's router owns the root of the one server (invoicer/ARCHITECTURE.md §6.1).
import adapter from "@sveltejs/adapter-static";
import {vitePreprocess} from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import {sveltekit} from "@sveltejs/kit/vite";
import {defineConfig} from "vitest/config";

/**
 * The invoicer's ports differ from Ledgeline's (5173/4173 → 5099), so both
 * e2e suites can run side by side (invoicer PLAN 3a): this SPA on 4273, the
 * `ledgeline-invoicer --server` on 5199.
 *
 * EVERYTHING outside `/app` goes to the server, not only `/api`: the invoice
 * page frames MBR's own page for the invoice (`/invoices/…/`, its assets under
 * `/.mbr/`), which the server gates on the session cookie `POST /api/session`
 * sets (PLAN A2). Proxied, the SPA, the API and MBR's pages are one origin,
 * exactly as in the app, so that `SameSite=Strict`, `Path=/` cookie rides on
 * the frame's requests. `ws` carries MBR's live-reload socket. `changeOrigin`
 * rewrites `Host` to 127.0.0.1:5199, which the server's Host guard requires.
 * Open http://localhost:4273/app/#token=<the token --server prints>.
 */
const SERVER = "http://127.0.0.1:5199";
const proxy = {"^/(?!app(?:/|$))": {target: SERVER, changeOrigin: true, ws: true}};

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
    // CodeMirror (the template editor's code field, PLAN 4d D2) is one lazy
    // chunk of ~506 kB (175 kB gzipped), fetched only when a template opens;
    // the limit sits just above it so any other chunk that size still warns.
    build: {chunkSizeWarningLimit: 520},
    server: {port: 4273, strictPort: true, proxy},
    preview: {port: 4273, strictPort: true, proxy},
    test: {
        // Inherited by both projects through `extends`, as in web/vite.config.ts.
        expect: {requireAssertions: true},
        env: {TZ: "America/Denver"},
        // The same split as web/ and packages/ui (see web/vite.config.ts): `unit`
        // runs in node and needs nothing; `components` mounts `*.svelte.test.ts`
        // in jsdom with the browser build of Svelte, through packages/ui's
        // shared setup (jsdom's missing layout APIs, Testing Library cleanup).
        // jsdom and Testing Library resolve from the hoisted root node_modules
        // (see package.json's "//devDependencies").
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
                    setupFiles: ["../../packages/ui/src/testing/componentSetup.ts"],
                },
            },
        ],
    },
});
