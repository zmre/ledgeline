// The editor's save and conflict rules, against a mocked server: our own save
// coming back is no conflict; an outside edit replaces a clean form and
// raises a conflict over a dirty one; Reload theirs and Keep mine settle it;
// a 409 `changed` save does the same; a sent invoice asks before re-booking.
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {emptyForm, type Computed, type EditorState} from "./editor";
import {InvoiceEditor} from "./invoiceEditor.svelte";
import ConflictNotice from "./components/ConflictNotice.svelte";
import {fireEvent, render, screen} from "@testing-library/svelte";

const COMPUTED: Computed = {text: "---\n…", changed: false, total: "$1,600.00", commodity: "$", table: null, table_error: null, problems: [], books: null};

const state = (revision: string, description = "Senior architect hours", status = "draft"): EditorState => ({
    path: "invoices/2026-11-17 Customer X Invoice 108.md",
    title: "2026-11-17 Customer X Invoice 108",
    number: "108",
    status,
    revision,
    form: emptyForm({
        customer: "[[Customer X]]",
        number: "108",
        date: "2026-11-17",
        table: {
            columns: [
                {header: "Description", role: "description"},
                {header: "Amount", role: "amount"},
            ],
            rows: [[description, "$1,600"]],
        },
    }),
    computed: COMPUTED,
});

type Reply = {status?: number; body: unknown};
let onDisk: EditorState;
let saveReply: ((body: Record<string, unknown>) => Reply) | null;
let saves: Record<string, unknown>[];

function respond(reply: Reply): Response {
    return new Response(JSON.stringify(reply.body), {status: reply.status ?? 200, headers: {"Content-Type": "application/json"}});
}

beforeEach(() => {
    vi.useFakeTimers();
    onDisk = state("r1");
    saveReply = null;
    saves = [];
    vi.stubGlobal(
        "fetch",
        vi.fn(async (path: string, init?: RequestInit) => {
            const method = init?.method ?? "GET";
            if (method === "GET" && path.endsWith("/form")) return respond({body: onDisk});
            if (method === "POST" && path.endsWith("/compute")) return respond({body: COMPUTED});
            if (method === "POST" && path === "/api/preview/draft") return respond({body: {url: "/.ledgeline-invoicer/preview?draft=x"}});
            if (method === "PUT" && path.endsWith("/form")) {
                const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
                saves.push(body);
                if (saveReply !== null) return respond(saveReply(body));
                if (body.revision !== onDisk.revision)
                    return respond({status: 409, body: {error: "changed on disk", code: "changed", revision: onDisk.revision}});
                onDisk = {...onDisk, revision: `${onDisk.revision}+`, form: body.form as EditorState["form"]};
                return respond({body: {written: true, path: onDisk.path, number: "108", revision: onDisk.revision, status: onDisk.status, rebooked: null}});
            }
            return respond({status: 404, body: {error: `no route ${method} ${path}`}});
        })
    );
});

afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

async function opened(): Promise<InvoiceEditor> {
    const editor = new InvoiceEditor();
    await editor.load("108");
    return editor;
}

function rename(editor: InvoiceEditor, text: string): void {
    const form = editor.form!;
    editor.edit({...form, table: {...form.table!, rows: [[text, "$1,600"]]}});
}

describe("InvoiceEditor", () => {
    it("is clean when loaded, dirty after an edit, clean once saved", async () => {
        const editor = await opened();
        expect(editor.dirty).toBe(false);
        rename(editor, "Architect hours");
        expect(editor.dirty).toBe(true);
        expect(editor.pending).toBe(true);
        expect(await editor.save()).toBe("108");
        expect(saves[0].revision).toBe("r1");
        expect(editor.dirty).toBe(false);
        expect(editor.revision).toBe("r1+");
        editor.dispose();
    });

    it("Discard drops the unsaved edits and is clean again", async () => {
        const editor = await opened();
        rename(editor, "Architect hours");
        expect(editor.dirty).toBe(true);
        editor.discard();
        expect(editor.dirty).toBe(false);
        expect(editor.form?.table?.rows[0][0]).toBe("Senior architect hours");
        expect(saves).toHaveLength(0);
        editor.dispose();
    });

    it("ignores its own save coming back as a change on disk", async () => {
        const editor = await opened();
        rename(editor, "Architect hours");
        await editor.save();
        await editor.diskChanged([onDisk.path]);
        expect(editor.conflict).toBeNull();
        expect(editor.form?.table?.rows[0][0]).toBe("Architect hours");
        editor.dispose();
    });

    it("shows an outside edit at once when nothing here is unsaved", async () => {
        const editor = await opened();
        onDisk = state("r2", "Edited elsewhere");
        await editor.diskChanged(["some/other.md"]);
        expect(editor.form?.table?.rows[0][0]).toBe("Senior architect hours");
        await editor.diskChanged([onDisk.path]);
        expect(editor.form?.table?.rows[0][0]).toBe("Edited elsewhere");
        expect(editor.revision).toBe("r2");
        expect(editor.conflict).toBeNull();
        editor.dispose();
    });

    it("raises a conflict over unsaved edits; Reload theirs drops mine", async () => {
        const editor = await opened();
        rename(editor, "Mine");
        onDisk = state("r2", "Theirs");
        await editor.diskChanged([]);
        expect(editor.conflict?.revision).toBe("r2");
        expect(editor.form?.table?.rows[0][0]).toBe("Mine");
        editor.reloadTheirs();
        expect(editor.form?.table?.rows[0][0]).toBe("Theirs");
        expect(editor.dirty).toBe(false);
        expect(editor.conflict).toBeNull();
        editor.dispose();
    });

    it("Keep mine saves over their revision, and only then", async () => {
        const editor = await opened();
        rename(editor, "Mine");
        onDisk = state("r2", "Theirs");
        // Saving over a stale revision is refused, and becomes the conflict.
        expect(await editor.save()).toBeNull();
        expect(editor.conflict?.revision).toBe("r2");
        expect(await editor.keepMine()).toBe("108");
        expect(saves.map((s) => s.revision)).toEqual(["r1", "r2"]);
        expect((onDisk.form.table?.rows[0] ?? [])[0]).toBe("Mine");
        editor.dispose();
    });

    it("asks before saving a sent invoice, which re-books it", async () => {
        onDisk = state("r1", "Senior architect hours", "sent");
        const editor = await opened();
        rename(editor, "Mine");
        saveReply = (body) =>
            body.rebook === true
                ? {body: {written: true, path: onDisk.path, number: "108", revision: "r9", status: "sent", rebooked: {changed: true}}}
                : {status: 409, body: {error: "confirm", code: "confirm-rebook"}};
        expect(await editor.save()).toBeNull();
        expect(editor.confirmRebook).toBe(true);
        expect(await editor.save({rebook: true})).toBe("108");
        expect(editor.confirmRebook).toBe(false);
        expect(editor.revision).toBe("r9");
        editor.dispose();
    });

    // TW-4: the server wrote the file, then re-booking it failed.
    const writeThen = (reply: Reply) => (body: Record<string, unknown>) => {
        onDisk = {...onDisk, revision: `${onDisk.revision}+`, form: body.form as EditorState["form"]};
        return reply;
    };
    const TODAY =
        "the invoice was saved, but re-booking it failed: hledger rejected the journal: `amount` doesn't balance; fix it, then Re-book (or `ledgeline-invoicer rebook 108`)";

    it("a save written but not re-booked is clean, says so, and its write is no conflict", async () => {
        onDisk = state("r1", "Senior architect hours", "sent");
        const editor = await opened();
        rename(editor, "Mine");
        saveReply = writeThen({status: 409, body: {error: TODAY, code: "refused"}});
        expect(await editor.save({rebook: true})).toBeNull();
        expect(editor.dirty).toBe(false);
        expect(editor.revision).toBe("r1+");
        expect(editor.confirmRebook).toBe(false);
        expect(editor.saveError).toMatch(/^Saved, but not re-booked: hledger rejected the journal: /);
        expect(editor.saveError).toMatch(/choose Re-book\.$/);
        expect(editor.saveError).not.toMatch(/`|ledgeline-invoicer|fix it, then/);
        // Its own write coming back.
        await editor.diskChanged([onDisk.path]);
        expect(editor.conflict).toBeNull();
        expect(editor.form?.table?.rows[0][0]).toBe("Mine");
        // The next save is over the new revision: no 409 `changed`.
        saveReply = null;
        rename(editor, "Mine again");
        expect(await editor.save()).toBe("108");
        expect(saves.at(-1)?.revision).toBe("r1+");
        editor.dispose();
    });

    it("a change event while the save is in flight is looked at once it is done", async () => {
        onDisk = state("r1", "Senior architect hours", "sent");
        const editor = await opened();
        rename(editor, "Mine");
        saveReply = (body) => {
            const reply = writeThen({status: 500, body: {error: TODAY, code: "hledger"}})(body);
            void editor.diskChanged([onDisk.path]); // the watcher, before the answer
            return reply;
        };
        await editor.save({rebook: true});
        await vi.waitFor(() => expect(saves).toHaveLength(1));
        await vi.runAllTimersAsync();
        expect(editor.conflict).toBeNull();
        expect(editor.dirty).toBe(false);
        editor.dispose();
    });

    it("trusts a new server's error that names the written revision, even if the file reads back differently", async () => {
        onDisk = state("r1", "Senior architect hours", "sent");
        const editor = await opened();
        rename(editor, "Mine");
        saveReply = () => {
            onDisk = state("r7", "Mine (normalised)", "sent");
            return {
                status: 422,
                body: {error: "Saved, but not re-booked: the books are closed through 2026-12-31.", code: "saved-not-rebooked", revision: "r7"},
            };
        };
        await editor.save({rebook: true});
        expect(editor.revision).toBe("r7");
        expect(editor.dirty).toBe(false);
        expect(editor.saveError).toBe(
            "Saved, but not re-booked: the books are closed through 2026-12-31. The file has your changes; the books don't yet. Fix the problem, then choose Re-book."
        );
        editor.dispose();
    });

    it("a save refused before anything is written stays unsaved, with the server's reason", async () => {
        onDisk = state("r1", "Senior architect hours", "sent");
        const editor = await opened();
        rename(editor, "Mine");
        saveReply = () => ({status: 409, body: {error: "it is dated on or before the books' closed-through date", code: "refused"}});
        await editor.save({rebook: true});
        expect(editor.dirty).toBe(true);
        expect(editor.revision).toBe("r1");
        expect(editor.saveError).toBe("It is dated on or before the books' closed-through date");
        editor.dispose();
    });

    it("a failed save that meets someone else's write keeps the edits, and the write is a conflict", async () => {
        const editor = await opened();
        rename(editor, "Mine");
        saveReply = () => {
            onDisk = state("r2", "Theirs");
            return {status: 500, body: {error: "the disk is full"}};
        };
        await editor.save();
        expect(editor.dirty).toBe(true);
        expect(editor.revision).toBe("r1");
        expect(editor.saveError).toBe("The disk is full");
        await editor.diskChanged([onDisk.path]);
        expect(editor.conflict?.revision).toBe("r2");
        editor.dispose();
    });

    it("drops a compute answer for a form edited since", async () => {
        const editor = await opened();
        rename(editor, "One");
        const asked = editor.compute(false);
        rename(editor, "Two");
        await asked;
        expect(editor.pending).toBe(true);
        await editor.compute(false);
        expect(editor.pending).toBe(false);
        editor.dispose();
    });
});

describe("COMPONENT ConflictNotice", () => {
    it("offers Reload theirs and Keep mine, and their text", async () => {
        const onReload = vi.fn();
        const onKeepMine = vi.fn();
        render(ConflictNotice, {theirs: "---\ntitle: theirs\n---\n", onReload, onKeepMine});
        expect(screen.getByRole("alert").textContent).toContain("changed on disk while you were editing");
        await fireEvent.click(screen.getByTestId("conflict-reload"));
        await fireEvent.click(screen.getByTestId("conflict-keep"));
        expect(onReload).toHaveBeenCalledOnce();
        expect(onKeepMine).toHaveBeenCalledOnce();
        expect(screen.getByText("Their file")).toBeTruthy();
    });
});
