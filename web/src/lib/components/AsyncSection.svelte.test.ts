// The shared error / loading / data tri-state, mounted.
//
// `routes/branchOrder.test.ts` already proves — by reading the source — that the
// error branch precedes the data branch and is not gated on the payload. What it
// cannot show is what a user actually READS when a request fails, and that
// became load-bearing when the engine started answering a bad REQUEST with a
// sentence rather than a status line: a `valueIn` that prices nothing in scope
// is a 400 naming the commodity and why no price reaches it.
//
// This fixture used to be a bad `holdings:` tag value, which was a 400 when this
// test was written and is not one any more — those five closed account tags are
// lenient now, and a bad value becomes an `account-tag` WARNING in the Problems
// drawer instead. Nothing about this component changed; the example just had to
// stop describing a response the engine can no longer send.
//
// That sentence has to survive two hops — `native.ts` preferring the response
// body over the status line, and this component composing the message — and
// neither hop had a test. This is the second one.

import {render, screen} from "@testing-library/svelte";
import {createRawSnippet} from "svelte";
import {describe, expect, it, vi} from "vitest";
import {ApiUnreachableError} from "$lib/api/client";
import {NativeApiUnavailableError, NATIVE_UNAVAILABLE_MESSAGE} from "$lib/api/native";
import type {DataView} from "$lib/stores/loadState";
import AsyncSection from "./AsyncSection.svelte";

/** The exact sentence the engine sends for an unpriceable `valueIn` (reports_api.rs). */
const BAD_REQUEST = "cannot value these holdings in 'XYZZY': no price directive or cost annotation connects any holding in scope to it";

// `unknown`, not `string`: AsyncSection is generic over its payload and infers
// `T` from every prop at once, so a narrower snippet fixes `T` to a type the
// `value: … | null` prop then contradicts.
const children = createRawSnippet<[unknown]>((value) => ({
    render: () => `<p data-testid="payload">${String(value())}</p>`,
}));

function mount(view: DataView, error: Error | null, onRetry = vi.fn()) {
    render(AsyncSection, {
        view,
        value: view === "data" ? "the report" : null,
        error,
        testid: "other-holdings-error",
        label: "other holdings",
        loadingLabel: "Loading other holdings",
        onRetry,
        children,
    });
    return onRetry;
}

describe("COMPONENT AsyncSection", () => {
    it("shows the engine's own sentence verbatim, so a bad `valueIn` is actionable", () => {
        mount("error", new ApiUnreachableError(BAD_REQUEST));
        const alert = screen.getByTestId("other-holdings-error");

        // Named by what failed, then the engine's sentence — not "responded 400".
        expect(alert.textContent).toContain("Couldn't load other holdings:");
        expect(alert.textContent).toContain(BAD_REQUEST);
        expect(alert.getAttribute("role")).toBe("alert");
    });

    it("offers Retry for it, because fixing the request is what makes the retry work", async () => {
        const onRetry = mount("error", new ApiUnreachableError(BAD_REQUEST));

        const retry = screen.getByRole("button", {name: "Retry"});
        retry.click();
        expect(onRetry).toHaveBeenCalledOnce();
    });

    it("shows a missing engine bare, with no Retry that could not help", () => {
        mount("error", new NativeApiUnavailableError(NATIVE_UNAVAILABLE_MESSAGE));

        expect(screen.getByTestId("other-holdings-error").textContent?.trim()).toBe(NATIVE_UNAVAILABLE_MESSAGE);
        expect(screen.queryByRole("button", {name: "Retry"})).toBeNull();
    });

    it("renders the payload only on the data branch, and a named spinner while loading", () => {
        mount("data", null);
        expect(screen.getByTestId("payload").textContent).toBe("the report");
    });

    it("names its spinner for screen readers rather than showing a bare animation", () => {
        mount("loading", null);

        expect(screen.getByLabelText("Loading other holdings")).toBeDefined();
        expect(screen.queryByTestId("payload")).toBeNull();
    });
});
