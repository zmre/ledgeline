<script>
    // Deliberately LEGACY Svelte: `export let` is how a Svelte 4 component
    // declared a prop, and runes mode refuses to compile it. Nothing renders
    // this — it exists so `compileMode.svelte.test.ts` can hand it to the test
    // pipeline and check that it is REFUSED, which is the only black-box proof
    // that `dynamicCompileOptions` in `svelte.config.js` reaches the components
    // project rather than only the app build.
    //
    // `svelte-check` and the editor's language server compile this in
    // AUTO-DETECT mode — they do not read the vite plugin's options — where a
    // component containing no runes is legacy and this is perfectly valid. That
    // gap between the two compilers is the whole reason the probe is needed, and
    // is why it does not show up as a type error.
    export let label = "legacy";
</script>

<p>{label}</p>
