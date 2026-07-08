import adapter from "@sveltejs/adapter-static";
import {vitePreprocess} from "@sveltejs/vite-plugin-svelte";

/** @type {import("@sveltejs/kit").Config} */
const config = {
    preprocess: vitePreprocess(),
    kit: {
        // Pure static SPA: every unknown path falls back to index.html and is routed client-side.
        adapter: adapter({fallback: "index.html"}),
    },
    vitePlugin: {
        // Force runes mode for our code while leaving node_modules libraries alone. Can be removed in svelte 6.
        dynamicCompileOptions({filename}) {
            return filename.split(/[/\\]/).includes("node_modules") ? undefined : {runes: true};
        },
    },
};

export default config;
