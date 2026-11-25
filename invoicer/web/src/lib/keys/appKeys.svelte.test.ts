// The invoicer's app-wide keys (PLAN 5k), mounted: the go-to chords and `n`,
// the help sheet in the invoicer's section order, `/` to the page's search
// and Escape in it, Shift+N for the New menu, and a dialog blinding the page.
import {render, screen, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";

const nav = vi.hoisted(() => ({url: new URL("http://localhost/app/invoices"), goto: vi.fn()}));
vi.mock("$app/state", () => ({
    page: {
        get url() {
            return nav.url;
        },
    },
}));
vi.mock("$app/navigation", () => ({goto: nav.goto}));
vi.mock("$app/paths", () => ({resolve: (path: string) => `/app${path}`}));

import KeyHelp from "#ui/keys/KeyHelp.svelte";
import {keymap} from "#ui/keys/keymap.svelte.js";
import KeysProbe from "#ui/testing/fixtures/KeysProbe.svelte";
import ConfirmDialog from "#lib/components/ConfirmDialog.svelte";
import InvoiceFilters from "#lib/components/InvoiceFilters.svelte";
import NewMenu from "#lib/components/NewMenu.svelte";
import {parseInvoiceQuery} from "#lib/listQuery.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {appLayer} from "./appKeys.js";
import {INVOICER_KEY_GROUPS} from "./groups.js";

describe("COMPONENT the invoicer's app keys", () => {
    listenForKeys();
    beforeEach(() => {
        nav.goto.mockReset();
        nav.url = new URL("http://localhost/app/invoices");
        render(KeysProbe, {props: {layerOf: appLayer}});
    });

    it("goes to each section with its g chord, and to a new invoice with n", async () => {
        for (const [letter, path] of [
            ["d", "/app/"],
            ["c", "/app/customers"],
            ["k", "/app/catalog"],
            ["s", "/app/snippets"],
            ["t", "/app/templates"],
        ]) {
            await press("g");
            await press(letter);
            expect(nav.goto).toHaveBeenLastCalledWith(path);
        }
        await press("n");
        expect(nav.goto).toHaveBeenLastCalledWith("/app/new");
    });

    it("stays put for the section it is on, unless the list is filtered", async () => {
        await press("g");
        await press("i");
        expect(nav.goto).not.toHaveBeenCalled();
        nav.url = new URL("http://localhost/app/invoices?status=open");
        await press("g");
        await press("i");
        expect(nav.goto).toHaveBeenCalledWith("/app/invoices");
    });

    it("lists every binding on the ? sheet, in the invoicer's section order", async () => {
        keymap.setGroupOrder(INVOICER_KEY_GROUPS);
        render(KeyHelp, {props: {note: "A note."}});
        await press("?");
        const sheet = await screen.findByTestId("key-help");
        const headings = within(sheet)
            .getAllByRole("heading", {level: 4})
            .map((h) => h.textContent);
        expect(headings).toEqual(["Global", "Go to"]);
        expect(within(sheet).getByText("Leave the field")).toBeTruthy();
        expect(within(sheet).getByText("Go to Catalog")).toBeTruthy();
        expect(within(sheet).getByText("A note.")).toBeTruthy();
        // No search box on the page: `/` is off, and off the sheet.
        expect(within(sheet).queryByText("Search this list")).toBeNull();
    });

    it("focuses the page's search with /, clears it with Escape, and leaves it with a second", async () => {
        const onChange = vi.fn();
        render(InvoiceFilters, {
            query: parseInvoiceQuery(new URLSearchParams("q=wingtip")),
            customers: [],
            onChange,
            onJump: vi.fn(),
            onClear: vi.fn(),
        });
        const box = screen.getByTestId("invoice-search") as HTMLInputElement;
        await press("/");
        expect(document.activeElement).toBe(box);
        // The `/` itself isn't typed into the box.
        expect(box.value).toBe("wingtip");

        await press("Escape", box);
        expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({q: ""}), {replace: true});
        await tick();
        expect(box.value).toBe("");
        await press("Escape", box);
        expect(document.activeElement).not.toBe(box);
    });

    it("leaves a field with Escape, so the page's keys work again", async () => {
        const field = document.createElement("input");
        document.body.append(field);
        await press("Escape", field);
        expect(document.activeElement).not.toBe(field);
        await press("n", document.body);
        expect(nav.goto).toHaveBeenCalledWith("/app/new");
        field.remove();
    });

    it("keeps focus in CodeMirror on Escape (Escape then Tab leaves it)", async () => {
        const editor = document.createElement("div");
        editor.className = "cm-editor";
        const content = document.createElement("div");
        content.contentEditable = "true";
        content.tabIndex = 0;
        editor.append(content);
        document.body.append(editor);
        await press("Escape", content);
        expect(document.activeElement).toBe(content);
        editor.remove();
    });

    it("opens the New menu on its first item with Shift+N", async () => {
        render(NewMenu, {choices: [{id: "invoice", label: "New invoice", href: "/app/new"}]});
        await press("N");
        await vi.waitFor(() => expect(document.activeElement).toBe(screen.getByTestId("new-menu-invoice")));
    });

    it("gives the page no keys while a dialog is open", async () => {
        const view = render(ConfirmDialog, {title: "Send?", confirmLabel: "Send", onConfirm: vi.fn(), onCancel: vi.fn()});
        await press("n", document.body);
        expect(nav.goto).not.toHaveBeenCalled();
        view.unmount();
        await tick();
        await press("n", document.body);
        expect(nav.goto).toHaveBeenCalledWith("/app/new");
    });
});
