// What the app bar's status span says, and what it says on hover.
//
// The span used to read the server URL, which answers a question nobody asks
// twice ("what port did I start it on?") and never the one that matters when
// several sets of books are open in several windows: WHICH LEDGER IS THIS. So
// the visible text is the engine's title for the journal, and NOTHING ELSE —
// a server that cannot name its journal (a plain hledger-web, or an engine
// older than `/api/journal`) gets no label at all, just the status dot. The URL
// is not a lesser answer to "which ledger is this", it is an answer to a
// different question, and putting it in this spot is what made the spot
// useless. Where-am-I-connected survives on the tooltip, beside the journal's
// file name, because that is what a reconnect needs and a reconnect is the only
// time it is wanted.
//
// Pure, and in its own module rather than inline in `+layout.svelte`, because
// every branch is a state that is awkward to stage in a mounted layout (no
// server configured; an engine that answers everything but this route; a title
// the engine sent as blank). Enumerating them here costs one import in the
// layout.

/** The status dot's states: "none" is no server configured, the rest are `journal.status`. */
export type ConnState = "none" | "idle" | "loading" | "ready" | "error";

/**
 * A string the engine sent that names nothing, read as if it had sent null.
 *
 * A title derived from a first-line comment can arrive as `"   "` (a bare `;`
 * on line one), and an empty label would leave the dot floating beside nothing
 * at all — strictly worse than the URL, which at least identifies the server.
 */
function named(value: string | null): string | null {
    const trimmed = value?.trim() ?? "";
    return trimmed === "" ? null : trimmed;
}

/**
 * The VISIBLE label: which ledger is on screen, or nothing.
 *
 * "not connected" when there is no server to ask, the journal's title when the
 * engine derived one, and the EMPTY STRING otherwise — the caller renders no
 * label at all, leaving the status dot to speak for itself. Deliberately no
 * fallback: this spot answers "which ledger is this", and a server address is
 * not a worse answer to that question, it is an answer to another one. Filling
 * the space with it is how the corner came to be ignored in the first place.
 */
export function connectionLabel(state: ConnState, title: string | null): string {
    if (state === "none") return "not connected";
    return named(title) ?? "";
}

/**
 * The `title=` tooltip: the error while there is one, else where this is coming
 * from.
 *
 * An error takes the whole tooltip because the label beside a red dot is a
 * ledger name, and the engine's sentence ("Cannot reach the Ledgeline engine at
 * …") is the only thing on screen that says why the Reconnect button appeared.
 * Otherwise it is `2026.journal — http://127.0.0.1:5000`: the file the title was
 * derived from, then the address, so a title that names an unexpected entity can
 * be traced to a file and a port without leaving the page.
 */
export function connectionTooltip(state: ConnState, file: string | null, serverUrl: string | null, error: string | null): string {
    if (state === "error") return error ?? "connection error";
    if (serverUrl === null) return "No hledger-web server configured";
    const name = named(file);
    return name === null ? serverUrl : `${name} — ${serverUrl}`;
}
