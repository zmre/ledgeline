// Assertions over the Insights (i) help (`InfoTip` fed from `metricHelp.ts`),
// shared by the dashboard's and the startup section's component tests.

import {within} from "@testing-library/svelte";
import {expect} from "vitest";
import {METRIC_HELP, type MetricHelp, type MetricId} from "#lib/reports/ui/insights/metricHelp.js";

/** `container` holds exactly one (i), and it shows `help`. */
export function expectHelp(container: HTMLElement, help: MetricHelp): void {
    const buttons = within(container).getAllByRole("button", {name: /^About /});
    expect(buttons).toHaveLength(1);
    expect(buttons[0].getAttribute("aria-label")).toBe(`About ${help.label}`);
    const tip = document.getElementById(buttons[0].getAttribute("aria-describedby") ?? "");
    expect(tip?.getAttribute("role")).toBe("tooltip");
    expect(tip?.textContent).toContain(help.title);
    expect(tip?.textContent).toContain(help.text);
    if (help.formula !== undefined) expect(tip?.textContent).toContain(help.formula);
}

/**
 * Every `insights-box-<id>` card on screen, by id. The suffix IS the help id,
 * so a card added without help (or under a new id) fails `expectEveryBoxHelped`.
 * Sub-testids (`-big`, `-small`, `-segments-toggle`) carry a hyphen and are skipped.
 */
export function boxes(): [string, HTMLElement][] {
    return [...document.querySelectorAll<HTMLElement>('[data-testid^="insights-box-"]')]
        .map((el): [string, HTMLElement] => [(el.dataset.testid ?? "").slice("insights-box-".length), el])
        .filter(([id]) => /^[a-z]+$/.test(id));
}

/** Each card on screen has help, keyed by its id; returns the ids seen. */
export function expectEveryBoxHelped(): string[] {
    const seen = boxes();
    for (const [id, el] of seen) {
        expect(Object.hasOwn(METRIC_HELP, id), `no help entry for box "${id}"`).toBe(true);
        expectHelp(el, METRIC_HELP[id as MetricId]);
    }
    return seen.map(([id]) => id);
}
