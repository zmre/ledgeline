// Setup for the `components` vitest project (see `vite.config.ts`).
//
// Nothing in the app imports anything from `$lib/testing`, so none of it reaches
// a bundle — it lives under `src/lib` rather than beside the config so that
// `tsc --noEmit` and `svelte-check` type-check it like every other module, and
// so a test can say `$lib/testing/…` instead of counting `../`s.

import {cleanup} from "@testing-library/svelte";
import {afterEach} from "vitest";

// jsdom implements no layout, and `scrollIntoView` is part of what it leaves
// out entirely — calling it throws `not a function` rather than doing nothing.
// Any component that keeps a keyboard cursor visible calls it, so stub it once
// here rather than in each test file.
//
// A no-op is the honest stub: with no layout engine there is no scroll position
// to move and nothing a test could truthfully assert about one. Scrolling is
// verified by pure unit tests over the arithmetic and by Playwright's
// `toBeInViewport`.
if (typeof Element !== "undefined" && Element.prototype.scrollIntoView === undefined) {
    Element.prototype.scrollIntoView = function scrollIntoView(): void {};
}

// jsdom leaves `matchMedia` out too, and Svelte's `MediaQuery` calls it at
// MODULE scope: `svelte/motion` builds a prefers-reduced-motion query on
// import, so any test importing a LayerChart component dies before its first
// assertion. Reporting "no preference" is the honest stub, and it is what the
// app assumes on a surface that has never asked.
if (typeof window !== "undefined" && window.matchMedia === undefined) {
    window.matchMedia = (query: string): MediaQueryList =>
        ({
            matches: false,
            media: query,
            onchange: null,
            addEventListener: () => {},
            removeEventListener: () => {},
            addListener: () => {},
            removeListener: () => {},
            dispatchEvent: () => false,
        }) as MediaQueryList;
}

// `bind:clientWidth` compiles to a ResizeObserver, which jsdom does not
// implement either. The stub never fires, so a measured dimension stays 0 and
// anything gated on one declines to draw. That is the correct outcome, not a
// workaround: with no layout engine there is no width to report, and a
// fabricated one would let a test assert geometry that nothing computed.
if (typeof globalThis.ResizeObserver === "undefined") {
    globalThis.ResizeObserver = class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
    };
}

// Testing Library only auto-registers its own teardown when `afterEach` is a
// GLOBAL, and this suite runs with vitest's default `globals: false`. Without
// this line every mounted component stays in the document and the next test's
// `getByTestId` finds two of everything — a failure that reads as a bug in the
// component rather than in the harness. Register it explicitly.
afterEach(cleanup);
