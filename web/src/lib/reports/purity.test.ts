// Enforces WP-06's purity rule: lib/reports/ (the engine, excluding the
// WP-07 ui/ subtree) must import zero Svelte/DOM/runtime-environment modules —
// it ports to Rust later. Engine sources may only use RELATIVE imports
// (./sibling or ../domain/...); test files may additionally use vitest and
// node builtins.

import {readdirSync, readFileSync} from "node:fs";
import {join} from "node:path";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";

const REPORTS_DIR = fileURLToPath(new URL(".", import.meta.url));

/** All .ts files under lib/reports/, excluding ui/ (WP-07's Svelte components live there). */
function sourceFiles(dir: string): string[] {
    return readdirSync(dir, {withFileTypes: true}).flatMap((entry) => {
        if (entry.isDirectory()) return entry.name === "ui" ? [] : sourceFiles(join(dir, entry.name));
        return entry.name.endsWith(".ts") ? [join(dir, entry.name)] : [];
    });
}

/** Static, re-export, side-effect, and dynamic import specifiers. */
function importSpecifiers(source: string): string[] {
    const re = /\bfrom\s+["']([^"']+)["']|\bimport\s*\(\s*["']([^"']+)["']\s*\)|\bimport\s+["']([^"']+)["']/g;
    const specifiers: string[] = [];
    for (const match of source.matchAll(re)) specifiers.push(match[1] ?? match[2] ?? match[3]);
    return specifiers;
}

const isTestFile = (file: string): boolean => file.endsWith(".test.ts") || file.endsWith("test-helpers.ts");
const TEST_ONLY_ALLOWED = /^(vitest$|node:)/;

describe("UNIT reports purity (no Svelte/DOM imports)", () => {
    it("engine sources import only relative modules; tests add only vitest/node builtins", () => {
        const files = sourceFiles(REPORTS_DIR);
        expect(files.length).toBeGreaterThan(0);

        const violations: string[] = [];
        for (const file of files) {
            for (const specifier of importSpecifiers(readFileSync(file, "utf8"))) {
                const relative = specifier.startsWith("./") || specifier.startsWith("../");
                const allowed = relative || (isTestFile(file) && TEST_ONLY_ALLOWED.test(specifier));
                if (!allowed) violations.push(`${file.slice(REPORTS_DIR.length)} imports "${specifier}"`);
            }
        }
        expect(violations).toEqual([]);
    });

    it("golden.test.ts (this suite's broadest importer) reaches api/domain only relatively", () => {
        // Belt-and-braces: the normalizer import must not drag in $lib aliases from here.
        const source = readFileSync(join(REPORTS_DIR, "golden.test.ts"), "utf8");
        expect(importSpecifiers(source).filter((s) => s.startsWith("$"))).toEqual([]);
    });
});
