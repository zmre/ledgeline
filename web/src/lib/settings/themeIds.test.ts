// The theme catalog lives in packages/ui (`#ui/theme/catalog`); this is the
// Ledgeline side of its contract.

import {readFileSync} from "node:fs";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {THEME_FAMILIES, THEME_PREF_IDS} from "#ui/theme/catalog.js";

// Three copies of the theme id list must agree: this catalog (what the SPA can
// render), the prefs wire (`THEME_IDS`, what the store accepts) and the engine's
// `THEMES` (what `ui-prefs.toml` / `ledgeline.toml` validate against).
describe("theme ids across the wire", () => {
    it("THEME_PREF_IDS is the prefs THEME_IDS set", async () => {
        const {THEME_IDS} = await import("#lib/settings/uiPrefs.js");
        expect([...THEME_PREF_IDS].sort()).toEqual([...THEME_IDS].sort());
    });

    it("THEME_PREF_IDS is the engine's THEMES set", () => {
        const rust = readFileSync(fileURLToPath(new URL("../../../../crates/ledgeline-server/src/ui_prefs.rs", import.meta.url)), "utf8");
        const block = /const THEMES: &\[&str\] = &\[([^\]]*)\];/.exec(rust)?.[1] ?? "";
        const ids = [...block.matchAll(/"([a-z-]+)"/g)].map((m) => m[1]);
        expect(ids.sort()).toEqual([...THEME_PREF_IDS].sort());
    });

    it("the engine's family table resolves every family as the catalog does", () => {
        const rust = readFileSync(fileURLToPath(new URL("../../../../crates/ledgeline-server/src/ui_prefs.rs", import.meta.url)), "utf8");
        const block = /const THEME_FAMILY_VARIANTS: &\[\(&str, &str, &str\)\] = &\[([\s\S]*?)\];/.exec(rust)?.[1] ?? "";
        const rows = [...block.matchAll(/\("([a-z-]+)", "([a-z-]+)", "([a-z-]+)"\)/g)].map((m) => [m[1], m[2], m[3]]);
        const expected = THEME_FAMILIES.filter((f) => f.variants.light !== undefined && f.variants.dark !== undefined).map((f) => [
            f.id,
            f.variants.light,
            f.variants.dark,
        ]);
        expect(rows).toEqual(expected);
    });
});
