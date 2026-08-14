// A source-text lint for one specific way a Svelte 5 `$effect` becomes an
// infinite loop.
//
// Reading files as text is unusual, and the justification is the same one
// `branchOrder.test.ts` gives — minus the part that has since expired.
//
// It used to say a behavioural test was not available at any price, because the
// vitest config had a single `node` project that excluded `*.svelte.test.ts`.
// That is no longer so: there is now a `components` project, and
// `AliasPanel.svelte.test.ts` mounts the panel this rule was written for and
// fails with the real `effect_update_depth_exceeded` if the latch is put back
// the way it was.
//
// This still earns its keep, for the reason a mount test cannot cover: it reads
// EVERY `.svelte` and `.svelte.ts` file under `src/`, including the ones nobody
// has written a component test for and the one somebody adds next week. A mount
// test proves one panel survives; this proves the shape is absent everywhere,
// and it does it in milliseconds without a DOM. The two are complements — a
// render test catches novel ways of writing a self-feeding effect that this
// regex will never know about, and this catches the known one in files no
// render test visits.
//
// # The bug this exists for
//
// `AliasPanel` seeded its form in an `$effect` latched on the chosen file:
//
//     let baseFile = $state<AliasFile | null>(null);
//     $effect(() => {
//         const chosen = files.find(...) ?? files[0];
//         if (baseFile === chosen) return;   // never true
//         baseFile = chosen;                 // so this always runs
//     });
//
// `$state` deep-proxies objects on assignment, so `baseFile` holds a PROXY and
// `chosen` is the raw object: `===` is never true and the guard never fires.
// `$state` is also tracked, so the effect reads a signal it writes and depends
// on itself. Svelte spins it until `effect_update_depth_exceeded` throws, and
// that error does not merely break the panel — it kills the whole app. Every
// nav link and every button stops responding, with nothing visible on screen
// saying why. It reached a human before anything caught it.
//
// # Why primitives are exempt
//
// Svelte only notifies when a value actually changes, and primitives are not
// proxied, so re-assigning the same string is a no-op and the loop cannot
// start. `EditRulesPanel` compares and assigns a `string | null` inside its own
// effect quite safely, and says so in a comment. Flagging it would train people
// to add suppressions, so the rule is deliberately narrow: it fires only on a
// latch whose declared type is not a primitive.
//
// The proven pattern for an object latch is a plain `let` — non-reactive, so it
// is neither proxied nor tracked. See `selectedFor` in `EditRulesPanel`.

import {describe, expect, it} from "vitest";
import {readFileSync, readdirSync} from "node:fs";
import {join} from "node:path";

/** Every `.svelte` and `.svelte.ts` file under `src/`. */
function sources(dir: string): string[] {
    return readdirSync(dir, {withFileTypes: true}).flatMap((entry) => {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) return sources(path);
        return entry.name.endsWith(".svelte") || entry.name.endsWith(".svelte.ts") ? [path] : [];
    });
}

/** The body of each `$effect(() => { … })` in `text`, brace-matched. */
function effectBodies(text: string): string[] {
    const bodies: string[] = [];
    for (const match of text.matchAll(/\$effect\(\(\)\s*=>\s*\{/g)) {
        let depth = 1;
        let i = match.index + match[0].length;
        const start = i;
        while (i < text.length && depth > 0) {
            if (text[i] === "{") depth += 1;
            else if (text[i] === "}") depth -= 1;
            i += 1;
        }
        bodies.push(text.slice(start, i - 1));
    }
    return bodies;
}

/** `$state` declarations whose annotated type is NOT a primitive, by name. */
function objectStates(text: string): string[] {
    const names: string[] = [];
    for (const match of text.matchAll(/let\s+(\w+)\s*=\s*\$state\s*(?:<([^>]*)>)?/g)) {
        const [, name, annotation] = match;
        const type = (annotation ?? "").replace(/\s|null|undefined|\|/g, "");
        // No annotation means inferred; treat it as an object, because the
        // dangerous case is exactly the one nobody wrote a type for.
        const primitive = type !== "" && /^(string|number|boolean|bigint)$/.test(type);
        if (!primitive) names.push(name);
    }
    return names;
}

describe("$effect latches", () => {
    it("never compare and assign a non-primitive $state inside their own effect", () => {
        const offenders = sources("src").flatMap((path) => {
            const text = readFileSync(path, "utf8");
            const states = objectStates(text);
            return effectBodies(text).flatMap((body) =>
                states
                    .filter((name) => new RegExp(`\\b${name}\\s*===`).test(body) && new RegExp(`\\b${name}\\s*=[^=]`).test(body))
                    .map((name) => `${path}: '${name}'`)
            );
        });

        // A proxy is never `===` its raw target, so such a guard never fires and
        // the effect re-triggers itself forever. Use a plain `let` for the latch
        // (see `selectedFor` in EditRulesPanel), or key it on a string.
        expect(offenders).toEqual([]);
    });
});
