import {describe, expect, it} from "vitest";
import {APP_NAME} from "./app.js";

describe("UNIT app", () => {
    it("names the app", () => {
        expect(APP_NAME).toBe("Ledgeline Invoicer");
    });
});
