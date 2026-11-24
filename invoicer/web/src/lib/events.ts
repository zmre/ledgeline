// Live reload (2026-11-23): the server's change events, `GET /api/events`
// (server-sent events), read with `fetch` because the route needs the bearer
// token and an `EventSource` can't send a header. The server sends `ready`
// once the stream is live and `changed` ({paths}) after each settled change on
// disk has been re-read and reconciled; the pages re-fetch what they show.
import {authHeaders} from "#ui/api/client.js";

/** One server-sent event. */
export interface ServerEvent {
    event: string;
    data: string;
}

/**
 * Split a server-sent-events text into whole events, keeping the unfinished
 * tail. Comments (`:` lines, the keep-alive) and fields other than `event` and
 * `data` are dropped; `data` lines join with a newline.
 */
export function parseEvents(buffer: string): {events: ServerEvent[]; rest: string} {
    const normalized = buffer.replace(/\r\n?/g, "\n");
    const blocks = normalized.split("\n\n");
    const rest = blocks.pop() ?? "";
    const events: ServerEvent[] = [];
    for (const block of blocks) {
        let event = "message";
        const data: string[] = [];
        for (const line of block.split("\n")) {
            if (line === "" || line.startsWith(":")) continue;
            const colon = line.indexOf(":");
            const field = colon === -1 ? line : line.slice(0, colon);
            const value = colon === -1 ? "" : line.slice(colon + 1).replace(/^ /, "");
            if (field === "event") event = value;
            else if (field === "data") data.push(value);
        }
        if (data.length > 0 || event !== "message") events.push({event, data: data.join("\n")});
    }
    return {events, rest};
}

/** The paths a `changed` event names (empty: "something; reload everything"). */
export function changedPaths(data: string): string[] {
    try {
        const body: unknown = JSON.parse(data);
        if (typeof body === "object" && body !== null && "paths" in body && Array.isArray(body.paths)) {
            return body.paths.filter((p): p is string => typeof p === "string");
        }
    } catch {
        // Not JSON: treat as "something changed".
    }
    return [];
}

/** How long to wait before reconnecting after `failures` failures in a row: 1 s, doubling, at most 10 s. */
export function retryDelay(failures: number): number {
    return Math.min(10_000, 1000 * 2 ** Math.max(0, failures - 1));
}

export interface ChangeListener {
    /** Files changed on disk (by the books folder); `[]` means reload everything. */
    onChange: (paths: string[]) => void;
}

/**
 * Listen to `GET /api/events` until the returned function is called,
 * reconnecting after a failure or the end of the stream (File → Open). A
 * reconnect that comes back `ready` reports `[]`: changes may have been missed.
 */
export function listenForChanges({onChange}: ChangeListener, fetcher: typeof fetch = fetch): () => void {
    const abort = new AbortController();
    let failures = 0;
    let connectedBefore = false;

    async function once(): Promise<void> {
        const response = await fetcher("/api/events", {
            headers: authHeaders({Accept: "text/event-stream"}, "/api/events"),
            signal: abort.signal,
            cache: "no-store",
        });
        if (!response.ok || response.body === null) throw new Error(`GET /api/events: HTTP ${response.status}`);
        const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
        let buffer = "";
        for (;;) {
            const {value, done} = await reader.read();
            if (done) return;
            const parsed = parseEvents(buffer + value);
            buffer = parsed.rest;
            for (const event of parsed.events) {
                if (event.event === "ready") {
                    failures = 0;
                    if (connectedBefore) onChange([]);
                    connectedBefore = true;
                } else if (event.event === "changed") {
                    onChange(changedPaths(event.data));
                }
            }
        }
    }

    async function loop(): Promise<void> {
        while (!abort.signal.aborted) {
            try {
                await once();
            } catch (error) {
                if (abort.signal.aborted) return;
                failures += 1;
                if (failures === 1) console.warn("live reload:", error);
            }
            if (abort.signal.aborted) return;
            await new Promise((resolve) => setTimeout(resolve, retryDelay(Math.max(1, failures))));
        }
    }

    void loop();
    return () => abort.abort();
}
