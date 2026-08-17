// Setup for the `components` vitest project (see `vite.config.ts`).
//
// Nothing in the app imports anything from `$lib/testing`, so none of it reaches
// a bundle — it lives under `src/lib` rather than beside the config so that
// `tsc --noEmit` and `svelte-check` type-check it like every other module, and
// so a test can say `$lib/testing/…` instead of counting `../`s.

import {cleanup} from "@testing-library/svelte";
import {afterEach} from "vitest";

// Testing Library only auto-registers its own teardown when `afterEach` is a
// GLOBAL, and this suite runs with vitest's default `globals: false`. Without
// this line every mounted component stays in the document and the next test's
// `getByTestId` finds two of everything — a failure that reads as a bug in the
// component rather than in the harness. Register it explicitly.
afterEach(cleanup);
