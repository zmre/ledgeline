import {describe, expect, it} from "vitest";
import {ago, autoPricesNote} from "./autoPrices";
import type {AutoPricesStatus} from "./pricesTypes";

const NOW = 1_800_000_000;

const status = (over: Partial<AutoPricesStatus>): AutoPricesStatus => ({
    enabled: true,
    lastRun: NOW - 2 * 3_600,
    lastOutcome: "updated",
    lastAdded: 3,
    backoffUntil: null,
    ...over,
});

describe("autoPricesNote", () => {
    it("says nothing while the updater is off, whatever it last did", () => {
        expect(autoPricesNote(status({enabled: false}), NOW)).toBeNull();
    });

    it("says when prices were last written", () => {
        expect(autoPricesNote(status({}), NOW)).toBe("Prices auto-updated 2 h ago");
    });

    it("says a check that found nothing missing was a check", () => {
        expect(autoPricesNote(status({lastOutcome: "up-to-date", lastRun: NOW - 300}), NOW)).toBe("Prices checked automatically 5 min ago");
        expect(autoPricesNote(status({lastOutcome: "nothing-new"}), NOW)).toBe("Prices checked automatically 2 h ago");
    });

    it("points at the button when there is no prices file yet", () => {
        expect(autoPricesNote(status({lastOutcome: "no-prices-file"}), NOW)).toBe("Click Update prices once to start automatic updates");
    });

    it("says when a rate-limit pause ends, and only while it lasts", () => {
        const paused = status({lastOutcome: "rate-limited", lastRun: NOW - 600, backoffUntil: NOW + 3_000});
        expect(autoPricesNote(paused, NOW)).toBe("Yahoo Finance asked for a pause; automatic updates resume in 50 min");
        expect(autoPricesNote(paused, NOW + 3_000)).toBe("Prices checked automatically 1 h ago");
    });

    it("reports a failure, and stays quiet about a lost race or no run yet", () => {
        expect(autoPricesNote(status({lastOutcome: "failed"}), NOW)).toBe("Automatic price update failed 2 h ago; it will retry");
        expect(autoPricesNote(status({lastOutcome: "conflict"}), NOW)).toBeNull();
        expect(autoPricesNote(status({lastOutcome: null, lastRun: null}), NOW)).toBeNull();
    });
});

describe("ago", () => {
    it("rounds down into minutes, hours and days", () => {
        expect(ago(5)).toBe("just now");
        expect(ago(119)).toBe("1 min ago");
        expect(ago(7_199)).toBe("1 h ago");
        expect(ago(86_400)).toBe("1 day ago");
        expect(ago(3 * 86_400)).toBe("3 days ago");
    });
});
