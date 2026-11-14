// The folded control bar (plan 42): below `sm` a button saying what the
// controls are set to opens them. jsdom has no container queries, so this
// checks the toggle's state and the classes that decide the panel's display.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {createRawSnippet} from "svelte";
import {describe, expect, it} from "vitest";
import FoldedControls from "./FoldedControls.svelte";

const children = createRawSnippet(() => ({render: () => `<label>Interval <select aria-label="Interval"></select></label>`}));

describe("COMPONENT FoldedControls", () => {
    it("says what the controls are set to and folds them until tapped", async () => {
        render(FoldedControls, {id: "demo-controls", summary: "Monthly · depth 3", testid: "demo", class: "gap-2", children});

        const toggle = screen.getByTestId("demo-toggle");
        expect(screen.getByTestId("demo-summary").textContent).toBe("ShowingMonthly · depth 3");
        expect(toggle.getAttribute("aria-controls")).toBe("demo-controls");
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
        const panel = screen.getByTestId("demo");
        expect(panel.id).toBe("demo-controls");
        // Hidden below `sm`, a flex row from `sm` up, styled by the caller.
        expect(panel.classList).toContain("hidden");
        expect(panel.classList).toContain("sm:flex");
        expect(panel.classList).toContain("gap-2");
        expect(screen.getByLabelText("Interval")).toBeTruthy();

        await fireEvent.click(toggle);
        expect(toggle.getAttribute("aria-expanded")).toBe("true");
        expect(panel.classList).toContain("flex");
        expect(panel.classList).not.toContain("hidden");
        expect(toggle.textContent).toContain("Done");
    });
});
