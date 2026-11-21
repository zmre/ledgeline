// The branch a data surface renders: `AsyncSection`'s `view`. Which one is the
// app's decision (Ledgeline: `dataView` in web/src/lib/stores/loadState.ts);
// this is only the vocabulary the two share.

/** The branch a surface should render. */
export type DataView = "error" | "loading" | "data";
