import {describe, expect, it} from "vitest";
import {isLink, linkName, toLink} from "./links.js";

describe("wikilinks without brackets", () => {
    it("shows a link's note name", () => {
        expect(linkName("[[Customer X]]")).toBe("Customer X");
        expect(linkName("[[Customer X|the customer]]")).toBe("Customer X");
        expect(linkName("Phase 1 build")).toBe("Phase 1 build");
        expect(linkName(null)).toBe("");
    });

    it("links a known note as it is spelled, and free text only when asked", () => {
        const titles = ["Customer X", "Estimate 12"];
        expect(toLink("customer x", titles)).toBe("[[Customer X]]");
        expect(toLink("Phase 1 build", titles)).toBe("Phase 1 build");
        expect(toLink("Northwind", titles, true)).toBe("[[Northwind]]");
        expect(toLink("[[Kept]]", titles)).toBe("[[Kept]]");
        expect(toLink("  ", titles, true)).toBe("");
        expect(isLink("[[A]]")).toBe(true);
        expect(isLink("A")).toBe(false);
    });
});
