// Clicking a period of the balance line scrolls the journal (plan 36
// amendment A1, decision 8). Its own file because the chart is mocked: jsdom
// has no layout, so layerchart's tooltip layer — what takes the click in a
// browser — never finds a bucket under the pointer. The stub hands the adapter
// a click on a known bucket instead; what the adapter does with it is the
// thing under test.

import {render} from "@testing-library/svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";
import {problems} from "#lib/stores/problems.svelte.js";
import {balanceSeries} from "#lib/testing/balanceSeriesFixture.js";
import BalanceTrend from "./BalanceTrend.svelte";

vi.mock("#lib/components/PeriodLineChart.svelte", async () => ({default: (await import("#lib/testing/PeriodLineChartStub.svelte")).default}));

const period = (i: number): HTMLButtonElement => document.querySelector(`[data-period="${i}"]`) as HTMLButtonElement;

beforeEach(() => problems.clearFocus());

describe("COMPONENT BalanceTrend click", () => {
    it("asks the journal to scroll to the period's newest transaction, as the Problems drawer does", () => {
        render(BalanceTrend, {series: balanceSeries()});
        period(1).click();
        expect(problems.focusRequest?.txnIndex).toBe(16);
        period(4).click();
        expect(problems.focusRequest?.txnIndex).toBe(12);
    });

    it("does nothing for a period in which nothing posted", () => {
        const onJump = vi.fn();
        render(BalanceTrend, {series: balanceSeries(), onJump});
        period(5).click();
        expect(onJump).not.toHaveBeenCalled();
    });
});
