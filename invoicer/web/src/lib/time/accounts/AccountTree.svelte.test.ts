// The Accounts tree: indented rows, resolved tags with where they come from,
// warnings and the missing-customer action, folding by button and by ←/→,
// and the row keys.
import {tick} from "svelte";
import {SvelteSet} from "svelte/reactivity";
import {fireEvent, render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {accountRow, accountsView} from "../testing.js";
import AccountTree from "./AccountTree.svelte";

function mount(rows = accountsView().accounts) {
    const folded = new SvelteSet<string>();
    const props = {
        rows,
        folded,
        onFold: vi.fn((account: string) => (folded.has(account) ? folded.delete(account) : folded.add(account))),
        onEdit: vi.fn(),
        onAddChild: vi.fn(),
        onRename: vi.fn(),
        onCreateCustomer: vi.fn(),
    };
    render(AccountTree, props);
    return props;
}

const row = (account: string): HTMLElement => screen.getAllByTestId("account-row").find((r) => r.dataset.account === account) as HTMLElement;
const shown = (): string[] => screen.getAllByTestId("account-row").map((r) => r.dataset.account ?? "");

describe("COMPONENT time AccountTree", () => {
    listenForKeys();

    it("shows each account's resolved tags and where they come from", () => {
        mount();
        const advisory = within(row("clients:acme:advisory"));
        expect(advisory.getByTestId("account-customer").textContent).toContain("Acme Corp");
        expect(advisory.getByTestId("account-customer").textContent).toContain("from clients:acme");
        expect(advisory.getByTestId("account-item").textContent).toContain("from clients:acme");
        expect(advisory.getByTestId("account-month").textContent).toBe("1h 04m");
        expect(within(row("clients:acme")).getByTestId("account-customer").textContent?.trim()).toBe("Acme Corp");
        expect(within(row("internal:old")).getByText("archived")).toBeDefined();
        expect(row("clients:acme:advisory").getAttribute("aria-level")).toBe("3");
    });

    it("flags a customer with no note, offering to create it, and shows warnings", async () => {
        const props = mount([
            accountRow({
                account: "clients:initech",
                customer: {value: "Initech Inc", from: "clients:initech", inherited: false},
                customer_found: false,
                billable: null,
                warnings: ["no customer note is called Initech Inc"],
            }),
        ]);
        expect(screen.getByTestId("account-warning").textContent).toBe("no customer note is called Initech Inc");
        await fireEvent.click(screen.getByTestId("account-create-customer"));
        expect(props.onCreateCustomer).toHaveBeenCalledWith(expect.objectContaining({account: "clients:initech"}));
    });

    it("folds a branch with its button", async () => {
        mount();
        await fireEvent.click(within(row("clients")).getByTestId("account-fold"));
        expect(shown()).toEqual(["clients", "internal", "internal:old"]);
    });

    it("→ unfolds then steps in, ← folds then steps out", async () => {
        const props = mount();
        await press("j"); // clients
        await press("ArrowLeft");
        expect(shown()).toEqual(["clients", "internal", "internal:old"]);
        await press("ArrowRight");
        expect(shown()).toContain("clients:acme");
        await press("ArrowRight"); // into clients:acme
        await press("ArrowRight"); // into advisory
        await tick();
        await press("Enter", document.activeElement as HTMLElement);
        expect(props.onEdit).toHaveBeenLastCalledWith(expect.objectContaining({account: "clients:acme:advisory"}));
        await press("ArrowLeft"); // a leaf: out to clients:acme
        await press("r");
        expect(props.onRename).toHaveBeenLastCalledWith(expect.objectContaining({account: "clients:acme"}));
        await press("a");
        expect(props.onAddChild).toHaveBeenLastCalledWith(expect.objectContaining({account: "clients:acme"}));
    });
});
