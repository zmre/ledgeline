// Mounting the rename list — the piece the "tiny thin columns" report was
// actually about.
//
// The claim under test is not "the CSS is right", which jsdom cannot see: it has
// no layout engine, so nothing here may ask how wide anything is. It is that the
// list is BUILT to be stackable — each pair is its own list item, each end of a
// pair is its own element, and neither end is shortened — because the previous
// version put the whole pair in one `<li>` inside a container that daisyUI was
// flowing into columns, and there was no arrangement of that markup which read
// well at any width.
//
// The accessible reading is asserted separately from the visible one. They are
// different in this component on purpose (see its header), which is exactly the
// kind of thing that rots silently if nothing pins it.

import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import {RENAMES_ONLY} from "$lib/testing/aliasFixtures";
import RenameList from "./RenameList.svelte";

describe("COMPONENT RenameList", () => {
    it("gives each rename its own row", () => {
        render(RenameList, {renames: RENAMES_ONLY.renames, testid: "renames"});

        expect(within(screen.getByTestId("renames")).getAllByRole("listitem")).toHaveLength(2);
    });

    it("reads as one string per pair, not two loose account names", () => {
        // A screen reader that met these as separate fragments — with a `→` most
        // of them pronounce as nothing — would be told two account names and not
        // that one becomes the other.
        render(RenameList, {renames: RENAMES_ONLY.renames, testid: "renames"});

        expect(screen.getByText("PW Roth IRA - 3077:cash → assets:morganstanley:pw-roth-ira:cash")).toBeDefined();
        expect(screen.getByText("PW Roth IRA - 3077 → assets:morganstanley:pw-roth-ira")).toBeDefined();
    });

    it("shows both ends in full, each in its own element", () => {
        // The whole point of stacking. Truncating either end loses the tail,
        // which for account names is where the difference between two of them
        // usually is.
        const {container} = render(RenameList, {renames: RENAMES_ONLY.renames, testid: "renames"});
        const [first] = within(screen.getByTestId("renames")).getAllByRole("listitem");
        const visible = [...first.querySelectorAll("[aria-hidden='true']")].map((node) => node.textContent);

        expect(visible).toContain("PW Roth IRA - 3077:cash");
        expect(visible).toContain("assets:morganstanley:pw-roth-ira:cash");
        // Nothing anywhere is allowed to be an ellipsis of an account name.
        expect(container.textContent).not.toContain("…");
    });

    it("renders nothing at all for an empty list", () => {
        render(RenameList, {renames: [], testid: "renames"});

        expect(within(screen.getByTestId("renames")).queryAllByRole("listitem")).toHaveLength(0);
    });
});
