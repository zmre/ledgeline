// Entries' state: the latest list wins, every write is followed by a fresh
// list (ids shift), a stale refusal reloads too, and picked lines that left
// the list are dropped.
import {describe, expect, it, vi} from "vitest";
import {EditorApiError} from "#lib/editorApi.js";
import {entry, list} from "../testing.js";
import {EntriesStore} from "./entriesStore.svelte.js";

const deferred = <T>() => {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((r) => (resolve = r));
    return {promise, resolve};
};

describe("COMPONENT time entries store", () => {
    it("shows only the latest request's answer", async () => {
        const slow = deferred<ReturnType<typeof list>>();
        const fetcher = vi
            .fn()
            .mockReturnValueOnce(slow.promise)
            .mockResolvedValueOnce(list([entry({id: "new@1"})]));
        const store = new EntriesStore(fetcher);
        const first = store.load("from=2026-10-01");
        await store.load("from=2026-11-01");
        slow.resolve(list([entry({id: "old@1"})]));
        await first;
        expect(store.list?.entries.map((e) => e.id)).toEqual(["new@1"]);
        expect(store.view).toBe("data");
    });

    it("fails only a first load; a list on screen stays", async () => {
        const fetcher = vi
            .fn()
            .mockRejectedValueOnce(new Error("down"))
            .mockResolvedValueOnce(list([entry()]))
            .mockRejectedValueOnce(new Error("down again"));
        const store = new EntriesStore(fetcher);
        await store.load("");
        expect(store.view).toBe("error");
        await store.retry();
        expect(store.view).toBe("data");
        vi.spyOn(console, "warn").mockImplementation(() => {});
        await store.load();
        expect(store.view).toBe("data");
    });

    it("a write reloads and drops the picks; a refusal keeps them", async () => {
        const fetcher = vi.fn().mockResolvedValue(list([entry({id: "a@1"}), entry({id: "a@2"})]));
        const store = new EntriesStore(fetcher);
        await store.load("q=x");
        store.toggle("a@1");
        const refused = await store.write(() => Promise.reject(new EditorApiError(422, "invalid", "no")));
        expect(refused).toMatchObject({ok: false, code: "invalid"});
        expect(fetcher).toHaveBeenCalledTimes(1);
        expect(store.selected.has("a@1")).toBe(true);
        expect(await store.write(() => Promise.resolve())).toEqual({ok: true});
        expect(fetcher).toHaveBeenCalledTimes(2);
        expect(fetcher).toHaveBeenLastCalledWith("q=x");
        expect(store.selected.size).toBe(0);
    });

    it("a conflict or a locked line reloads what is on screen", async () => {
        const fetcher = vi.fn().mockResolvedValue(list([entry()]));
        const store = new EntriesStore(fetcher);
        await store.load("");
        await store.write(() => Promise.reject(new EditorApiError(409, "external-change", "the file changed")));
        expect(fetcher).toHaveBeenCalledTimes(2);
    });

    it("picks a range, unpicks it when all are picked, and forgets lines that left", async () => {
        const fetcher = vi
            .fn()
            .mockResolvedValueOnce(list([entry({id: "a@1"}), entry({id: "a@2", person: "sam"})]))
            .mockResolvedValueOnce(list([entry({id: "a@2"})]));
        const store = new EntriesStore(fetcher);
        await store.load("");
        expect([...store.people]).toEqual(["pat", "sam"]);
        store.toggleRange(["a@1", "a@2"]);
        expect(store.selected.size).toBe(2);
        store.toggleRange(["a@1", "a@2"]);
        expect(store.selected.size).toBe(0);
        store.toggleRange(["a@1", "a@2"]);
        await store.load();
        expect([...store.selected]).toEqual(["a@2"]);
        // A new query starts with nothing picked.
        fetcher.mockResolvedValueOnce(list([]));
        await store.load("q=other");
        expect(store.selected.size).toBe(0);
    });
});
