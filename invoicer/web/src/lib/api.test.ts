import {describe, expect, it} from "vitest";
import {appearanceFrom} from "./api.js";

describe("UNIT api", () => {
    it("reads the shared theme and scale from GET /api/ui-prefs", () => {
        expect(appearanceFrom({prefs: {theme: "catppuccin", uiScale: 125}})).toEqual({theme: "catppuccin", uiScale: 125});
    });

    it("ignores a body without them", () => {
        expect(appearanceFrom({prefs: {theme: 3, uiScale: "big"}})).toBeNull();
        expect(appearanceFrom({prefs: {}})).toBeNull();
    });
});
