// The JS ↔ Swift contract (plans/42-mobile-app.md), checked from the JS side.
//
// The iOS app names every piece of the bridge in Swift STRING LITERALS: the
// message handler, the `window.ledgelineNative` calls, the injected global and
// class, the message types and haptic styles. A rename on either side compiles
// and silently breaks the app, so this reads the Swift sources and asserts the
// literals this module uses are all there. Skipped (not failed) on a checkout
// with no apple/ios sources.

import {readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {fileURLToPath} from "node:url";
import {describe, expect, it} from "vitest";
import {MESSAGE_HANDLER, NATIVE_API, NATIVE_CLASS, NATIVE_GLOBAL} from "./detect";

const IOS_ROOT = fileURLToPath(new URL("../../../../apple/ios", import.meta.url));

function swiftFiles(dir: string): string[] {
    let entries: string[];
    try {
        entries = readdirSync(dir);
    } catch {
        return [];
    }
    return entries.flatMap((name) => {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) return swiftFiles(path);
        return name.endsWith(".swift") ? [path] : [];
    });
}

const files = swiftFiles(IOS_ROOT);
const swift = files.map((path) => readFileSync(path, "utf8")).join("\n");

describe.skipIf(files.length === 0)("the Swift side of the bridge", () => {
    it("registers the message handler the page posts to", () => {
        expect(swift).toContain(`"${MESSAGE_HANDLER}"`);
    });

    it("calls every window.ledgelineNative method the page installs", () => {
        for (const method of ["navigate", "action", "back", "state"]) {
            expect(swift, `${NATIVE_API}.${method}`).toContain(`${NATIVE_API}.${method}`);
        }
    });

    it("injects the global and the class native mode keys off", () => {
        expect(swift).toContain(NATIVE_GLOBAL);
        expect(swift).toContain(NATIVE_CLASS);
    });

    it("knows every message type and haptic style the page sends", () => {
        for (const literal of ["page", "share", "haptic", "light", "success", "warning"]) {
            expect(swift, `"${literal}"`).toContain(`"${literal}"`);
        }
    });
});
