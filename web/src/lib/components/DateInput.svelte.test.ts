// The date box for a date that may be unset.
//
// WebKit paints TODAY into an empty `<input type="date">`, so an unset date read
// as today. The hint and the hidden native text are the fix; the e2e suite runs
// Chromium, which never showed the bug, so this file is where it is pinned.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {datePattern} from "#lib/format/datePattern.js";
import DateInput from "./DateInput.svelte";

const HIDE_NATIVE = "not-focus:[&::-webkit-datetime-edit]:opacity-0";

const box = (): HTMLInputElement => screen.getByLabelText("When") as HTMLInputElement;
const hint = (): HTMLElement | null => screen.queryByTestId("date-unset");

describe("COMPONENT DateInput", () => {
    it.each([null, ""])("an unset date (%o) shows the locale's date pattern and hides the native text", (value) => {
        render(DateInput, {value, "aria-label": "When"});
        expect(hint()?.textContent).toBe(datePattern());
        expect(box().className).toContain(HIDE_NATIVE);
    });

    it("a set date shows itself, with no hint over it", () => {
        render(DateInput, {value: "2028-01-01", "aria-label": "When"});
        expect(box().value).toBe("2028-01-01");
        expect(hint()).toBeNull();
        expect(box().className).not.toContain(HIDE_NATIVE);
    });

    it("REGRESSION: the hint follows the BOX, not a caller that has not committed yet", async () => {
        // One-way `value`, as the debounced filter bar passes it.
        render(DateInput, {value: null, "aria-label": "When"});
        await fireEvent.input(box(), {target: {value: "2028-01-01"}});
        expect(hint()).toBeNull();

        await fireEvent.input(box(), {target: {value: ""}});
        expect(hint()).not.toBeNull();
    });

    it("passes the caller's attributes and handlers through to the input", async () => {
        const onchange = vi.fn();
        render(DateInput, {value: "", "aria-label": "When", disabled: true, class: "w-36", size: "xs", onchange});
        expect(box().type).toBe("date");
        expect(box().disabled).toBe(true);
        expect(box().className).toContain("w-36");
        expect(box().className).toContain("input-xs");

        await fireEvent.change(box(), {target: {value: "2028-01-01"}});
        expect(onchange).toHaveBeenCalledOnce();
    });
});
