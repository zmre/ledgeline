// A source-text lint for daisyUI's alert layout trap, in the same spirit — and
// for the same reason — as `effectLatch.test.ts` next door.
//
// # The bug
//
// daisyUI v5 styles `.alert` as
//
//     display: grid;
//     grid-template-columns: auto;
//     grid-auto-flow: column;
//     &:has(:nth-child(2)) { grid-template-columns: auto minmax(auto, 1fr) }
//
// It is a GRID, and it flows its children into COLUMNS. `flex-col` sets
// `flex-direction`, which a grid container ignores entirely, so writing
//
//     <div class="alert alert-info flex-col items-start gap-2">
//         <span class="font-semibold">…headline…</span>
//         <ul>…a list of account renames…</ul>
//         <p>…a paragraph explaining why…</p>
//     </div>
//
// does not stack anything. It produces three narrow columns side by side, the
// first sized `auto` and the rest fighting over `minmax(auto,1fr)` — which is
// how the dry run's alias notices came to be described as "nearly unreadable
// … things are put in tiny thin columns side by side making it hard to read or
// scan". The fix is one token: `flex flex-col`, which overrides `display:grid`
// and makes `flex-col` mean what it was written to mean.
//
// Two files in this codebase already carried a comment warning about exactly
// this (`EditRulesPanel`'s conflict alert, and `RulesFileList` for the same trap
// on `menu` items, where daisyUI grids the item wrapper). Six other alerts had
// the defect anyway. A comment in one file cannot stop that; this can.
//
// # What it can and cannot see
//
// It reads static `class="…"` attributes only. A class computed entirely inside
// `class={…}` is invisible to it, as is an alert with several children that
// never asked to stack at all — the rule is "if you said `flex-col` you must
// also say `flex`", not "every alert must stack", because a two-child alert
// with a message and a button is a perfectly good row. Narrow on purpose: a
// lint that fires on the reasonable case is a lint people learn to suppress.

import {describe, expect, it} from "vitest";
import {readFileSync, readdirSync} from "node:fs";
import {join} from "node:path";

/** Every `.svelte` file under `src/`. */
function components(dir: string): string[] {
    return readdirSync(dir, {withFileTypes: true}).flatMap((entry) => {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) return components(path);
        return entry.name.endsWith(".svelte") ? [path] : [];
    });
}

/** The whitespace-separated tokens of every static `class="…"` in `text`. */
function classLists(text: string): string[][] {
    return [...text.matchAll(/class="([^"]*)"/g)].map((match) => match[1].split(/\s+/).filter((token) => token !== ""));
}

describe("daisyUI alerts", () => {
    it("never say flex-col without flex, which a grid ignores", () => {
        const offenders = components("src").flatMap((path) =>
            classLists(readFileSync(path, "utf8"))
                .filter((tokens) => tokens.includes("alert") && tokens.includes("flex-col") && !tokens.includes("flex"))
                .map((tokens) => `${path}: '${tokens.join(" ")}'`)
        );

        // `.alert` is `display:grid; grid-auto-flow:column`, so `flex-col` alone
        // is inert and the children lay out as narrow side-by-side columns.
        // Write `flex flex-col`.
        expect(offenders).toEqual([]);
    });

    it("finds the class lists it is supposed to be reading", () => {
        // The rule above passes trivially if the scan is broken — a regex that
        // matches nothing reports no offenders. This pins that it still sees the
        // alerts it was written for, and that the fixed spelling is what it sees.
        const stacked = components("src")
            .flatMap((path) => classLists(readFileSync(path, "utf8")))
            .filter((tokens) => tokens.includes("alert") && tokens.includes("flex-col"));

        expect(stacked.length).toBeGreaterThan(5);
        expect(stacked.every((tokens) => tokens.includes("flex"))).toBe(true);
    });
});
