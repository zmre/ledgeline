import tailwindcss from "@tailwindcss/vite";
import {sveltekit} from "@sveltejs/kit/vite";
import {defineConfig} from "vitest/config";

export default defineConfig({
    plugins: [tailwindcss(), sveltekit()],
    test: {
        expect: {requireAssertions: true},
        // Pin the zone so a date bug cannot hide behind whoever's ambient TZ. CI
        // ran UTC and dev runs America/Denver, so the suite had never executed in
        // a negative-offset zone — exactly where `new Date("YYYY-MM-DD")` (UTC
        // midnight, read via local getters) silently lands on the previous day.
        // Denver makes CI and dev identical and keeps that trap armed.
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
        ],
    },
});
