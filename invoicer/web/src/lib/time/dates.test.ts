// The one date module every time page shares: calendar arithmetic on UTC
// midnights and the words dates are shown in.
import {describe, expect, it} from "vitest";
import {addDays, addMonths, dayHeading, dayMonth, daysBetween, longMonthYear, monthRange, rangeText, shiftMonth, weekdayName} from "./dates.js";

describe("UNIT time dates", () => {
    it("does date arithmetic by calendar, never by parsing text", () => {
        expect(daysBetween("2026-11-25", "2026-11-26")).toBe(1);
        expect(daysBetween("2026-10-31", "2026-11-26")).toBe(26);
        expect(addDays("2026-03-08", 1)).toBe("2026-03-09");
        expect(addMonths("2026-03-31", -1)).toBe("2026-02-28");
        expect(shiftMonth("2026-01", -1)).toBe("2025-12");
        expect(monthRange("2026-02-10")).toEqual({from: "2026-02-01", to: "2026-02-28"});
    });

    it("names dates the way each page shows them", () => {
        expect(weekdayName("2026-11-26")).toBe("Thu");
        expect(dayMonth("2026-11-26")).toBe("Nov 26");
        expect(dayHeading("2026-11-26")).toBe("Thu, Nov 26");
        expect(longMonthYear("2026-11")).toBe("November 2026");
        expect(longMonthYear("2026-10-31")).toBe("October 2026");
        expect(rangeText("2026-10-01", "2026-10-31")).toBe("Oct 1 – 31, 2026");
    });
});
