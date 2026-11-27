// The sidebar, mounted: the New split button and its keyboard menu, its
// sections with their counts (each saying what it counts), and the open
// books by name, with the basis as plain text.
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {createRawSnippet, type ComponentProps} from "svelte";
import {describe, expect, it, vi} from "vitest";
import Sidebar from "./Sidebar.svelte";

// Plain `tsc` sees a .svelte module's default export only, so the type comes from the props.
type SidebarLink = ComponentProps<typeof Sidebar>["links"][number];

const links: SidebarLink[] = [
    {
        id: "invoices",
        href: "/app/invoices",
        label: "Invoices",
        active: true,
        counts: [
            {id: "overdue", value: 2, label: "overdue", tone: "error"},
            {id: "count", value: 1, label: "open, not yet due"},
        ],
        title: "2 overdue · 1 open, not yet due · 1 draft, not counted",
    },
    {id: "customers", href: "/app/customers", label: "Customers", active: false, counts: [{id: "count", value: 6, label: "customers"}]},
    {id: "catalog", href: "/app/catalog", label: "Catalog", active: false},
];
const newChoices = [
    {id: "invoice", label: "New invoice", href: "/app/new"},
    {id: "customer", label: "New customer", href: "/app/customers/new"},
    {id: "snippet", label: "New snippet", href: "/app/snippets?new=1"},
];
const books = {
    journal: "books.journal",
    title: "Acme Books",
    basis: "accrual" as const,
    documents: 8,
    invoices: {all: 8, drafts: 1, open: 3, overdue: 1},
    customers: 6,
    catalog: 4,
    snippets: 2,
    nextNumber: "2005",
};

describe("COMPONENT Sidebar", () => {
    it("marks the current section and says what each count counts (H17)", () => {
        render(Sidebar, {links, books});
        const invoices = screen.getByTestId("sidebar-invoices");
        expect(invoices.getAttribute("href")).toBe("/app/invoices");
        expect(invoices.getAttribute("aria-current")).toBe("page");
        expect(invoices.getAttribute("title")).toBe("2 overdue · 1 open, not yet due · 1 draft, not counted");
        const overdue = within(invoices).getByTestId("sidebar-invoices-overdue");
        expect(overdue.textContent).toBe("2 overdue");
        expect(overdue.className).toContain("badge-error");
        expect(overdue.getAttribute("title")).toBe("2 overdue");
        const notDue = within(invoices).getByTestId("sidebar-invoices-count");
        expect(notDue.textContent).toBe("1 open, not yet due");
        expect(notDue.className).not.toContain("badge");
        expect(screen.getByTestId("sidebar-customers-count").textContent).toBe("6 customers");
        expect(screen.getByTestId("sidebar-customers-count").getAttribute("title")).toBe("6 customers");
        expect(screen.queryByTestId("sidebar-catalog-count")).toBeNull();
        expect(screen.queryByTestId("sidebar-customers-overdue")).toBeNull();
        // No New button without choices; no "New invoice" section link.
        expect(screen.queryByTestId("new-menu")).toBeNull();
        expect(screen.queryByRole("link", {name: "New invoice"})).toBeNull();
    });

    it("puts the New split button at the top: New invoice, the rest in its menu (H20)", async () => {
        render(Sidebar, {links, books, newChoices});
        const main = screen.getByTestId("new-main");
        expect(main.textContent).toBe("New invoice");
        expect(main.getAttribute("href")).toBe("/app/new");
        const toggle = screen.getByTestId("new-toggle");
        expect(toggle.getAttribute("aria-haspopup")).toBe("menu");
        expect(toggle.getAttribute("aria-expanded")).toBe("false");
        expect(screen.queryByRole("menu")).toBeNull();

        // ArrowDown on the main part opens the menu on its first item.
        await fireEvent.keyDown(main, {key: "ArrowDown"});
        const menu = screen.getByRole("menu");
        const items = within(menu).getAllByRole("menuitem");
        expect(items.map((i) => i.textContent)).toEqual(["New invoice", "New customer", "New snippet"]);
        expect(items[2].getAttribute("href")).toBe("/app/snippets?new=1");
        expect(document.activeElement).toBe(items[0]);
        expect(toggle.getAttribute("aria-expanded")).toBe("true");
        expect(toggle.getAttribute("aria-controls")).toBe(menu.id);

        // Arrows move and wrap; Home and End jump.
        await fireEvent.keyDown(menu, {key: "ArrowDown"});
        expect(document.activeElement).toBe(items[1]);
        await fireEvent.keyDown(menu, {key: "ArrowUp"});
        await fireEvent.keyDown(menu, {key: "ArrowUp"});
        expect(document.activeElement).toBe(items[2]);
        await fireEvent.keyDown(menu, {key: "Home"});
        expect(document.activeElement).toBe(items[0]);
        await fireEvent.keyDown(menu, {key: "End"});
        expect(document.activeElement).toBe(items[2]);

        // Escape closes it, focus back on the arrow.
        await fireEvent.keyDown(menu, {key: "Escape"});
        expect(screen.queryByRole("menu")).toBeNull();
        expect(document.activeElement).toBe(toggle);

        // Enter on the arrow opens it; ArrowUp opens on the last item.
        await fireEvent.keyDown(toggle, {key: "Enter"});
        expect(document.activeElement).toBe(within(screen.getByRole("menu")).getAllByRole("menuitem")[0]);
        await fireEvent.keyDown(screen.getByRole("menu"), {key: "Escape"});
        await fireEvent.keyDown(toggle, {key: "ArrowUp"});
        expect(document.activeElement?.textContent).toBe("New snippet");

        // A click outside closes it.
        await fireEvent.pointerDown(document.body);
        expect(screen.queryByRole("menu")).toBeNull();
        // A click on the arrow opens it.
        await fireEvent.click(toggle);
        expect(screen.getByRole("menu")).toBeDefined();
    });

    it("names the open books, with the basis as text and no document count", () => {
        render(Sidebar, {links, books});
        const foot = screen.getByTestId("sidebar-books");
        expect(within(foot).getByTestId("sidebar-books-name").textContent).toBe("Acme Books");
        expect(foot.textContent).toContain("books.journal");
        const basis = within(foot).getByTestId("sidebar-basis");
        expect(basis.textContent).toBe("Accrual basis");
        expect(basis.className).not.toContain("badge");
        expect(basis.closest("button")).toBeNull();
        expect(foot.textContent).not.toContain("documents");
    });

    it("falls back to the journal's name, and says when no books are open", () => {
        const {unmount} = render(Sidebar, {links, books: {...books, title: null}});
        expect(screen.getByTestId("sidebar-books-name").textContent).toBe("books.journal");
        unmount();
        render(Sidebar, {links: [{...links[0], active: false}], books: null});
        expect(screen.getByTestId("sidebar-books").textContent).toContain("No books open");
        expect(screen.getByTestId("sidebar-invoices").getAttribute("aria-current")).toBeNull();
    });

    it("ends with Settings, after every section, and marks it while it is open", () => {
        render(Sidebar, {links, books, settings: {id: "settings", href: "/app/settings", label: "Settings", active: true}});
        const all = screen.getAllByRole("link");
        const settings = screen.getByTestId("sidebar-settings");
        expect(all[all.length - 1]).toBe(settings);
        expect(settings.getAttribute("href")).toBe("/app/settings");
        expect(settings.getAttribute("aria-current")).toBe("page");
    });

    it("puts a plain group's links under its heading, before Settings, and the lamp above them all (plan 46)", () => {
        const groups = [{id: "more", title: "More", links: [{id: "extra", href: "/app/extra", label: "Extra", narrowLabel: "More extra", active: true}]}];
        const lamp = createRawSnippet(() => ({render: () => '<div data-testid="lamp-probe">lamp</div>'}));
        render(Sidebar, {links, books, groups, lamp, settings: {id: "settings", href: "/app/settings", label: "Settings", active: false}});
        expect(screen.getByTestId("sidebar-group-more").textContent).toBe("More");
        const all = screen.getAllByRole("link").map((a) => a.dataset.testid);
        expect(all).toEqual(["sidebar-invoices", "sidebar-customers", "sidebar-catalog", "sidebar-extra", "sidebar-settings"]);
        expect(screen.getByTestId("sidebar-extra").getAttribute("aria-current")).toBe("page");
        // The narrow bar has no headings: a link may say more there.
        expect(screen.getByTestId("sidebar-extra").textContent?.trim()).toBe("More extraExtra");
        const probe = screen.getByTestId("lamp-probe");
        const nav = screen.getByRole("navigation", {name: "Sections"});
        expect(probe.compareDocumentPosition(nav) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    describe("a collapsible group (the Library)", () => {
        const library = (open: boolean, active: string | null = null, onToggle = vi.fn()) => ({
            id: "library",
            title: "Library",
            open,
            onToggle,
            links: [
                {id: "snippets", href: "/app/snippets", label: "Snippets", active: active === "snippets", counts: [{id: "count", value: 2, label: "snippets"}]},
                {id: "time-accounts", href: "/app/time/accounts", label: "Time accounts", active: active === "time-accounts"},
            ],
        });
        const toggle = () => screen.getByRole("button", {name: "Library"});
        const shown = () => screen.queryAllByRole("link").map((a) => a.dataset.testid);

        it("is a button saying whether it is open; closed, its links are hidden", async () => {
            const onToggle = vi.fn();
            const {rerender} = render(Sidebar, {links, books, groups: [library(false, null, onToggle)]});
            expect(toggle().getAttribute("aria-expanded")).toBe("false");
            expect(toggle().dataset.testid).toBe("sidebar-group-library");
            expect(shown()).toEqual(["sidebar-invoices", "sidebar-customers", "sidebar-catalog"]);
            // aria-controls names every link it shows or hides.
            for (const id of (toggle().getAttribute("aria-controls") ?? "").split(" ")) expect(document.getElementById(id)?.hidden).toBe(true);

            await fireEvent.click(toggle());
            expect(onToggle).toHaveBeenLastCalledWith(true);
            await rerender({links, books, groups: [library(true, null, onToggle)]});
            expect(toggle().getAttribute("aria-expanded")).toBe("true");
            expect(shown()).toEqual(["sidebar-invoices", "sidebar-customers", "sidebar-catalog", "sidebar-snippets", "sidebar-time-accounts"]);
            expect(screen.getByTestId("sidebar-snippets-count").textContent).toBe("2 snippets");

            // Pressed again, it closes.
            await fireEvent.click(toggle());
            expect(onToggle).toHaveBeenLastCalledWith(false);
        });

        it("opens by itself while one of its pages is current, and stays closed there once closed", async () => {
            const onToggle = vi.fn();
            const {rerender} = render(Sidebar, {links, books, groups: [library(false, "time-accounts", onToggle)]});
            expect(toggle().getAttribute("aria-expanded")).toBe("true");
            expect(screen.getByTestId("sidebar-time-accounts").getAttribute("aria-current")).toBe("page");

            await fireEvent.click(toggle());
            expect(onToggle).toHaveBeenLastCalledWith(false);
            expect(toggle().getAttribute("aria-expanded")).toBe("false");
            expect(screen.queryByTestId("sidebar-time-accounts")?.closest("li")?.hidden).toBe(true);

            // Another of its pages opens it again.
            await rerender({links, books, groups: [library(false, "snippets", onToggle)]});
            expect(toggle().getAttribute("aria-expanded")).toBe("true");
        });
    });
});
