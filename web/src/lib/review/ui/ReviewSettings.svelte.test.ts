// ReviewSettings is the home of both the review and the Find Matches
// settings (plan 35 D8): the journal's ▾ shows both groups, each overlay's
// menu only its own, and a number typed out of range is clamped by the store.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {afterEach, describe, expect, it} from "vitest";
import {MATCH_LOOKBACK, MATCH_TRANSIT_DAYS, MATCH_WINDOW, settings} from "#lib/stores/settings.svelte.js";
import ReviewSettings from "./ReviewSettings.svelte";

afterEach(() => {
    settings.matchWindow = MATCH_WINDOW;
    settings.matchLookback = MATCH_LOOKBACK;
    settings.matchTransitDays = MATCH_TRANSIT_DAYS;
});

describe("COMPONENT ReviewSettings", () => {
    it("shows both groups by default, and only its own in each overlay", () => {
        const all = render(ReviewSettings);
        expect(screen.getByText("Find Matches")).toBeDefined();
        expect(screen.getByLabelText("How many days back to review")).toBeDefined();
        all.unmount();
        const review = render(ReviewSettings, {show: "review"});
        expect(screen.queryByText("Find Matches")).toBeNull();
        review.unmount();
        render(ReviewSettings, {show: "matches"});
        expect(screen.queryByLabelText("How many days back to review")).toBeNull();
        expect(screen.getByTestId("setting-matchWindow")).toBeDefined();
    });

    it("commits the match settings, clamped to their ranges", async () => {
        render(ReviewSettings, {show: "matches"});
        await fireEvent.change(screen.getByTestId("setting-matchWindow"), {target: {value: "90"}});
        await fireEvent.change(screen.getByTestId("setting-matchLookback"), {target: {value: "0"}});
        await fireEvent.change(screen.getByTestId("setting-matchTransitDays"), {target: {value: "30"}});
        expect(settings.matchWindow).toBe(60);
        expect(settings.matchLookback).toBe(0);
        expect(settings.matchTransitDays).toBe(30);
    });
});
