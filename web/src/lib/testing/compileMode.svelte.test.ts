// The components project must compile components EXACTLY as production does, or
// every test in it is evidence about a build nobody ships.
//
// This repo forces runes on via `dynamicCompileOptions` in `svelte.config.js`,
// which is an option of `@sveltejs/vite-plugin-svelte` — not of the Svelte
// compiler, and not of vitest. It therefore applies only where that plugin is in
// the pipeline. The components project inherits it by extending
// `./vite.config.ts` (same `sveltekit()` plugin, same `svelte.config.js`), which
// is true by construction right up until someone gives the project its own
// plugin list to fix something unrelated. Then the compiler falls back to
// AUTO-DETECT: components using runes still get runes, and anything that does
// not silently compiles as Svelte 4, where a plain `let` is reactive and an
// `$effect` does not exist. The tests would keep passing while testing a
// different language.
//
// Auto-detect and forced-runes differ observably in exactly one cheap way: what
// they refuse. `export let` is legal in auto-detect and a compile error under
// runes, so asking the pipeline to compile one settles the question.

import {describe, expect, it} from "vitest";

describe("the components project compiles with runes forced, like the app build", () => {
    it("refuses `export let`, which only runes mode refuses", async () => {
        await expect(import("./fixtures/LegacyPropsProbe.svelte")).rejects.toThrow(/runes mode/);
    });
});
