import tailwindcss from "@tailwindcss/vite";
import {sveltekit} from "@sveltejs/kit/vite";
import {defineConfig} from "vitest/config";

export default defineConfig({
    plugins: [tailwindcss(), sveltekit()],
    test: {
        // Everything in this block is inherited by BOTH projects below, because
        // each one says `extends: "./vite.config.ts"` — that pulls this file's
        // `test` options in (minus `projects`, which vitest strips to avoid
        // recursing). Put anything that must hold suite-wide HERE, not in a
        // project, or the two halves drift apart.
        expect: {requireAssertions: true},
        // Pin the zone so a date bug cannot hide behind whoever's ambient TZ. CI
        // ran UTC and dev runs America/Denver, so the suite had never executed in
        // a negative-offset zone — exactly where `new Date("YYYY-MM-DD")` (UTC
        // midnight, read via local getters) silently lands on the previous day.
        // Denver makes CI and dev identical and keeps that trap armed.
        env: {TZ: "America/Denver"},
        // TWO projects, split by what they need to run rather than by what they
        // test. `unit` needs nothing (no DOM, no engine, no browser) and stays
        // the fast default; `components` pays for a jsdom document so a `.svelte`
        // file can actually be mounted.
        //
        // The split exists because logic tests were passing while the screen was
        // wrong. Every pure function behind the New Transactions flow was green
        // while that screen showed a spinner and "Reading the file" before any
        // file had been dropped, and disabled the destination and balance fields:
        // the LOGIC was right and the components were handed the wrong values,
        // which is a claim no pure-function test can make. Separately, an
        // `$effect` loop in `AliasPanel` threw `effect_update_depth_exceeded` and
        // froze the whole app — no navigation anywhere, no visible error — and
        // nothing in the suite mounted a component to notice.
        //
        // Run both: `vitest run`. Run one: `vitest run --project=unit` or
        // `--project=components`.
        projects: [
            {
                extends: "./vite.config.ts",
                test: {
                    name: "unit",
                    environment: "node",
                    include: ["src/**/*.{test,spec}.{js,ts}"],
                    // Routed to `components` instead — see below.
                    exclude: ["src/**/*.svelte.{test,spec}.{js,ts}"],
                },
            },
            {
                extends: "./vite.config.ts",
                // Svelte ships a server (SSR, string-rendering) build and a
                // client (mountable, reactive) one, chosen by export condition,
                // and node's defaults pick the server one. Required, not
                // cosmetic: without it every test here dies on
                // `lifecycle_function_unavailable` — `mount(...)` is not
                // available on the server. It fails loudly rather than
                // producing quietly inert components, which is the good outcome,
                // but it fails. Set on the project and not in the root config so
                // the app build is untouched.
                resolve: {conditions: ["browser"]},
                test: {
                    name: "components",
                    // jsdom, not vitest's browser mode. Browser mode is the more
                    // faithful renderer and we would prefer it, but Chromium
                    // cannot launch in this environment at all
                    // (`bootstrap_check_in … Permission denied`), which would
                    // leave these tests unrunnable locally and unrunnable for any
                    // agent working in the sandbox — i.e. runnable only on CI,
                    // which is precisely where a test stops being consulted while
                    // you work. jsdom renders Svelte 5 runes correctly (they are
                    // plain JS reactivity over DOM APIs, not a rendering feature)
                    // and needs no browser download. The cost is real and worth
                    // naming: jsdom has no layout engine, so nothing here may
                    // assert on geometry, visibility-by-overlap, or CSS-computed
                    // anything. Assert on structure and on the values a component
                    // was handed. That is what both bugs were.
                    environment: "jsdom",
                    include: ["src/**/*.svelte.{test,spec}.{js,ts}"],
                    setupFiles: ["./src/lib/testing/componentSetup.ts"],
                },
            },
        ],
    },
});
