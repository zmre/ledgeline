import {describe, expect, it, vi} from "vitest";
import {changedPaths, listenForChanges, parseEvents, retryDelay} from "./events.js";

describe("UNIT events", () => {
    it("splits server-sent events, keeping the unfinished tail", () => {
        const {events, rest} = parseEvents('event: ready\ndata: {}\n\n: keep-alive\n\nevent: changed\ndata: {"paths":["a.md"]}\n\nevent: chan');
        expect(events).toEqual([
            {event: "ready", data: "{}"},
            {event: "changed", data: '{"paths":["a.md"]}'},
        ]);
        expect(rest).toBe("event: chan");
        expect(parseEvents("data: a\r\ndata: b\r\n\r\n").events).toEqual([{event: "message", data: "a\nb"}]);
    });

    it("reads a changed event's paths, anything odd meaning everything", () => {
        expect(changedPaths('{"paths":["invoices/x.md","books.journal"]}')).toEqual(["invoices/x.md", "books.journal"]);
        expect(changedPaths('{"paths":[]}')).toEqual([]);
        expect(changedPaths("nonsense")).toEqual([]);
    });

    it("backs off 1 s, doubling, to 10 s", () => {
        expect([1, 2, 3, 4, 5, 9].map(retryDelay)).toEqual([1000, 2000, 4000, 8000, 10_000, 10_000]);
    });

    it("reports each change, and a reconnect as everything", async () => {
        const encoder = new TextEncoder();
        const stream = (chunks: string[]) =>
            new Response(
                new ReadableStream<Uint8Array>({
                    start(controller) {
                        for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
                        controller.close();
                    },
                }),
                {headers: {"Content-Type": "text/event-stream"}}
            );
        vi.useFakeTimers();
        const fetcher = vi
            .fn<typeof fetch>()
            .mockResolvedValueOnce(stream(["event: ready\ndata: {}\n\nevent: chan", 'ged\ndata: {"paths":["invoices/x.md"]}\n\n']))
            .mockResolvedValueOnce(stream(["event: ready\ndata: {}\n\n"]))
            .mockImplementation(() => new Promise(() => {}));
        const seen: string[][] = [];
        const stop = listenForChanges({onChange: (paths) => seen.push(paths)}, fetcher);
        await vi.waitFor(() => expect(seen).toEqual([["invoices/x.md"]]));
        await vi.advanceTimersByTimeAsync(1000);
        await vi.waitFor(() => expect(seen).toEqual([["invoices/x.md"], []]));
        expect(fetcher.mock.calls[0][0]).toBe("/api/events");
        stop();
        vi.useRealTimers();
    });
});
