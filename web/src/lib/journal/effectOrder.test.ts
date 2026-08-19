// Two structural properties of TransactionTable's effects that no render test
// can reach, guarded by reading the source — the same technique, and the same
// justification, as `routes/branchOrder.test.ts` and `routes/effectLatch.test.ts`.
//
// jsdom has no layout engine, so `scroller.scrollTop` is always 0 and a mount
// test cannot tell a correct scroll reset from a broken one. But WHICH REACTIVE
// SOURCE an effect subscribes to is the entire bug here, and that is plainly
// visible in the text.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";

const TABLE = fileURLToPath(new URL("./TransactionTable.svelte", import.meta.url));
const source = readFileSync(TABLE, "utf8");

const between = (from: string, to: string): string => source.slice(source.indexOf(from), source.indexOf(to));

describe("UNIT TransactionTable effect order", () => {
    it("REGRESSION: resets scroll on a FILTER change, not on every txns identity change", () => {
        // `txns` is a fresh array after every refresh — including the one
        // `editing.patch` fires on success — so keying the reset on it sent the
        // user back to row 0 after every inline edit. Invisible while editing
        // requires a click; intolerable the moment a keystroke can cycle the
        // status of the row under the cursor.
        const effect = between("// EFFECT 1", "// EFFECT 2");

        expect(effect).toContain("void filters.value;");
        expect(effect).not.toContain("void txns;");
    });

    it("keeps the problems-drawer effect declared after the scroll reset", () => {
        // Declaration order is load-bearing and the component already carries a
        // comment saying so: a drawer jump and a filter change can land in the
        // same flush, and the jump has to win.
        expect(source.indexOf("// EFFECT 1")).toBeLessThan(source.indexOf("// EFFECT 2"));
    });
});
