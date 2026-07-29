// `createResource` is now the only implementation of the two invariants that
// used to be re-derived in four stores, so it is the only place they can be
// tested — and the only place they can regress.

import {describe, expect, it} from "vitest";
import {createResource} from "./resource.svelte";

/** A fetcher whose responses are resolved by hand, so responses can land out of order. */
function deferred<T>(): {promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void} {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return {promise, resolve, reject};
}

describe("UNIT createResource drops superseded responses", () => {
    it("keeps the NEWEST request's answer when an older one lands last", async () => {
        // The whole point of the monotonic token. Without it the slow first
        // request wins simply by finishing second, and the surface shows an
        // answer to a question the user has already moved on from.
        const first = deferred<string>();
        const second = deferred<string>();
        const pending = [first, second];
        const resource = createResource<string, string>(() => pending.shift()!.promise);

        const a = resource.load("http://engine", "first");
        const b = resource.load("http://engine", "second");

        second.resolve("SECOND");
        await b;
        first.resolve("FIRST");
        await a;

        expect(resource.value).toBe("SECOND");
        expect(resource.query).toBe("second");
        expect(resource.status).toBe("ready");
    });

    it("does not let a superseded FAILURE overwrite the newest success", async () => {
        // The stale check has to guard the catch block too. It did in all four
        // copies, but that is exactly the kind of detail a fifth copy omits.
        const first = deferred<string>();
        const second = deferred<string>();
        const pending = [first, second];
        const resource = createResource<string, string>(() => pending.shift()!.promise);

        const a = resource.load("http://engine", "first");
        const b = resource.load("http://engine", "second");

        second.resolve("SECOND");
        await b;
        first.reject(new Error("the abandoned request failed"));
        await a;

        expect(resource.status).toBe("ready");
        expect(resource.value).toBe("SECOND");
        expect(resource.error).toBeNull();
    });
});

describe("UNIT createResource ties each payload to the query that produced it (FE-1)", () => {
    it("publishes the value and its query in one step", async () => {
        const resource = createResource<string, string>((_url, query) => Promise.resolve(`payload for ${query}`));
        await resource.load("http://engine", "bs");
        expect(resource.query).toBe("bs");
        expect(resource.value).toBe("payload for bs");
    });

    it("leaves the OLD payload and the OLD query in place after a failure", async () => {
        // This pair is what lets a surface tell "stale but labelled" from
        // "answers the current question": the payload survives so it can still
        // be rendered, but it is still tagged with the request it answered, so
        // the page can refuse to show it under a new label.
        const resource = createResource<string, string>((_url, query) =>
            query === "is" ? Promise.reject(new Error("connection refused")) : Promise.resolve(`payload for ${query}`)
        );
        await resource.load("http://engine", "bs");
        await resource.load("http://engine", "is");

        expect(resource.status).toBe("error");
        expect(resource.value).toBe("payload for bs");
        expect(resource.query).toBe("bs");
        expect(resource.query).not.toBe("is");
    });

    it("wraps a non-Error rejection rather than storing it raw", async () => {
        const resource = createResource<string, string>(() => Promise.reject("a bare string"));
        await resource.load("http://engine", "bs");
        expect(resource.error).toBeInstanceOf(Error);
        expect(resource.error?.message).toBe("a bare string");
    });
});

describe("UNIT createResource.view puts failure ahead of stale data (FE-5)", () => {
    it("is 'loading' before anything has been fetched", () => {
        const resource = createResource<string, string>(() => Promise.resolve("x"));
        expect(resource.view).toBe("loading");
    });

    it("is 'data' after a success", async () => {
        const resource = createResource<string, string>(() => Promise.resolve("x"));
        await resource.load("http://engine", "q");
        expect(resource.view).toBe("data");
    });

    it("is 'error' after a failed REFETCH, even though a payload is still held", async () => {
        // The exact case the old hand-written chains got wrong: they asked for
        // an error AND `payload === null`, so once anything had loaded the
        // error branch was unreachable and a failed refetch silently kept
        // serving the previous answer.
        const resource = createResource<string, string>((_url, query) => (query === "bad" ? Promise.reject(new Error("500")) : Promise.resolve("x")));
        await resource.load("http://engine", "good");
        await resource.load("http://engine", "bad");

        expect(resource.value).not.toBeNull();
        expect(resource.view).toBe("error");
    });
});
