import {describe, expect, it} from "vitest";
import {bucketEnd, bucketKey, bucketLabel, bucketStart, compareISO, lastNBuckets, today} from "./periods";

describe("UNIT reports/periods", () => {
    describe("bucketKey", () => {
        it("buckets daily/monthly/quarterly/yearly by string math", () => {
            expect(bucketKey("2026-07-08", "daily")).toBe("2026-07-08");
            expect(bucketKey("2026-07-08", "monthly")).toBe("2026-07");
            expect(bucketKey("2026-01-31", "quarterly")).toBe("2026-Q1");
            expect(bucketKey("2026-03-31", "quarterly")).toBe("2026-Q1");
            expect(bucketKey("2026-04-01", "quarterly")).toBe("2026-Q2");
            expect(bucketKey("2026-10-01", "quarterly")).toBe("2026-Q4");
            expect(bucketKey("2026-07-08", "yearly")).toBe("2026");
        });

        it("computes ISO weeks, including week-year boundaries", () => {
            // Expectations cross-checked against ISO-8601 week numbering.
            expect(bucketKey("2026-07-08", "weekly")).toBe("2026-W28"); // Wednesday
            expect(bucketKey("2026-01-01", "weekly")).toBe("2026-W01"); // Thursday
            expect(bucketKey("2025-12-29", "weekly")).toBe("2026-W01"); // Monday belongs to next week-year
            expect(bucketKey("2024-12-29", "weekly")).toBe("2024-W52"); // Sunday
            expect(bucketKey("2024-12-30", "weekly")).toBe("2025-W01"); // Monday belongs to next week-year
            expect(bucketKey("2021-01-01", "weekly")).toBe("2020-W53"); // Friday belongs to previous week-year
            expect(bucketKey("2020-12-31", "weekly")).toBe("2020-W53");
            expect(bucketKey("2019-12-30", "weekly")).toBe("2020-W01");
            expect(bucketKey("2020-02-29", "weekly")).toBe("2020-W09"); // leap day
            expect(bucketKey("2015-12-28", "weekly")).toBe("2015-W53"); // 53-week year
            expect(bucketKey("2016-01-03", "weekly")).toBe("2015-W53");
            expect(bucketKey("2016-01-04", "weekly")).toBe("2016-W01");
        });
    });

    describe("bucketLabel", () => {
        it("labels each key format", () => {
            expect(bucketLabel("2026-07")).toBe("Jul 2026");
            expect(bucketLabel("2026-01")).toBe("Jan 2026");
            expect(bucketLabel("2026-Q3")).toBe("Q3 2026");
            expect(bucketLabel("2026-W05")).toBe("W05 2026");
            expect(bucketLabel("2026")).toBe("2026");
            expect(bucketLabel("2026-07-08")).toBe("2026-07-08");
        });
    });

    describe("bucketStart / bucketEnd", () => {
        it("handles month ends including leap years", () => {
            expect(bucketEnd("2024-02")).toBe("2024-02-29"); // leap
            expect(bucketEnd("2023-02")).toBe("2023-02-28");
            expect(bucketEnd("2100-02")).toBe("2100-02-28"); // century non-leap
            expect(bucketEnd("2000-02")).toBe("2000-02-29"); // 400-year leap
            expect(bucketEnd("2026-01")).toBe("2026-01-31");
            expect(bucketEnd("2026-04")).toBe("2026-04-30");
            expect(bucketEnd("2026-12")).toBe("2026-12-31");
            expect(bucketStart("2026-12")).toBe("2026-12-01");
        });

        it("handles quarter and year bounds", () => {
            expect(bucketEnd("2026-Q1")).toBe("2026-03-31");
            expect(bucketEnd("2024-Q1")).toBe("2024-03-31");
            expect(bucketEnd("2026-Q2")).toBe("2026-06-30");
            expect(bucketEnd("2026-Q3")).toBe("2026-09-30");
            expect(bucketEnd("2026-Q4")).toBe("2026-12-31");
            expect(bucketStart("2026-Q3")).toBe("2026-07-01");
            expect(bucketEnd("2026")).toBe("2026-12-31");
            expect(bucketStart("2026")).toBe("2026-01-01");
            expect(bucketEnd("2026-07-08")).toBe("2026-07-08");
            expect(bucketStart("2026-07-08")).toBe("2026-07-08");
        });

        it("bounds ISO weeks Monday..Sunday across year boundaries", () => {
            expect(bucketStart("2026-W28")).toBe("2026-07-06");
            expect(bucketEnd("2026-W28")).toBe("2026-07-12");
            expect(bucketStart("2026-W01")).toBe("2025-12-29");
            expect(bucketEnd("2026-W01")).toBe("2026-01-04");
            expect(bucketStart("2020-W01")).toBe("2019-12-30");
            expect(bucketEnd("2020-W53")).toBe("2021-01-03");
        });

        it("rejects unrecognized keys", () => {
            expect(() => bucketEnd("garbage")).toThrow(RangeError);
            expect(() => bucketStart("2026-Q5")).toThrow(RangeError);
        });
    });

    describe("lastNBuckets", () => {
        it("walks monthly buckets across year boundaries, oldest → newest", () => {
            expect(lastNBuckets("2026-02-15", "monthly", 4)).toEqual(["2025-11", "2025-12", "2026-01", "2026-02"]);
        });

        it("walks quarterly and yearly buckets", () => {
            expect(lastNBuckets("2026-02-15", "quarterly", 5)).toEqual(["2025-Q1", "2025-Q2", "2025-Q3", "2025-Q4", "2026-Q1"]);
            expect(lastNBuckets("2026-02-15", "yearly", 3)).toEqual(["2024", "2025", "2026"]);
        });

        it("walks weekly buckets across week-year boundaries", () => {
            expect(lastNBuckets("2026-01-01", "weekly", 3)).toEqual(["2025-W51", "2025-W52", "2026-W01"]);
            expect(lastNBuckets("2021-01-04", "weekly", 3)).toEqual(["2020-W52", "2020-W53", "2021-W01"]);
        });

        it("walks daily buckets across the leap day", () => {
            expect(lastNBuckets("2024-03-01", "daily", 3)).toEqual(["2024-02-28", "2024-02-29", "2024-03-01"]);
        });

        it("returns an empty list for n ≤ 0", () => {
            expect(lastNBuckets("2026-07-08", "monthly", 0)).toEqual([]);
        });
    });

    describe("compareISO / today", () => {
        it("compares lexically", () => {
            expect(compareISO("2026-01-31", "2026-02-01")).toBe(-1);
            expect(compareISO("2026-02-01", "2026-02-01")).toBe(0);
            expect(compareISO("2026-02-02", "2026-02-01")).toBe(1);
        });

        it("today() is a well-formed local ISO date", () => {
            expect(today()).toMatch(/^\d{4}-\d{2}-\d{2}$/);
        });
    });
});
