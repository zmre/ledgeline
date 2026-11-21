import {describe, expect, it} from "vitest";
import {isVariantId} from "#ui/theme/catalog.js";
import {resolveTheme} from "#ui/theme/resolve.js";
import {APP_NAME, defaultAppearance} from "./app.js";

describe("UNIT app", () => {
    it("names the app", () => {
        expect(APP_NAME).toBe("Ledgeline Invoicer");
    });

    it("defaults to a shared theme that resolves in both OS schemes", () => {
        const {theme, uiPrefsReady} = defaultAppearance();
        expect(isVariantId(resolveTheme(theme, false))).toBe(true);
        expect(isVariantId(resolveTheme(theme, true))).toBe(true);
        expect(uiPrefsReady).toBe(false);
    });
});
