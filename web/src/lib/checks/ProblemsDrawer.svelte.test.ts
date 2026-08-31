// The drawer's two kinds of row.
//
// Every finding used to be anchored to a transaction, so every row in this
// drawer was a button that scrolled the journal table to it. `account-tag` is
// the first finding anchored to an `account` DIRECTIVE instead, and there is no
// row to scroll to — so the rendering has to branch, and the branch has to be
// visible from the outside:
//
//   anchored   -> a <button>, showing the transaction's date and description
//   unanchored -> plain text, showing the ACCOUNT NAME, with no click target
//
// The second half is the part worth a test. "Not clickable" is easy to get
// wrong in a way nothing else notices: a button that calls a handler which
// early-returns still LOOKS clickable, still takes focus, still invites the
// click, and does nothing when it lands. Asserting on the element rather than on
// the handler is what keeps that honest.

import {render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {Problem} from "$lib/checks/engine";
import type {Transaction} from "$lib/domain/types";

const requestFocus = vi.fn();

// The drawer reads three stores and SvelteKit's navigation. Only `problems` and
// `journal` carry anything this file asserts on; the rest are stubbed to keep
// the component mountable.
//
// `drawerOpen` is part of that state now, because the drawer only renders its
// contents when it is OPEN — a closed drawer is hidden by daisyUI with
// `visibility: hidden` rather than `display: none`, so building 21k findings
// into it was real layout work for a panel nobody had asked for. Every
// assertion about a ROW therefore has to open it first; `show()` does that, and
// the last test pins the empty-when-closed half.
const state = {problems: [] as Problem[], txns: [] as Transaction[], drawerOpen: true};

vi.mock("$app/navigation", () => ({goto: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => path}));
vi.mock("$app/state", () => ({page: {url: new URL("http://localhost/")}}));
vi.mock("$lib/stores/filters.svelte", () => ({
    filters: {value: {from: null, to: null}, setRange: vi.fn()},
}));
vi.mock("$lib/stores/journal.svelte", () => ({
    journal: {
        get txns() {
            return state.txns;
        },
    },
}));
vi.mock("$lib/stores/problems.svelte", () => ({
    problems: {
        get all() {
            return state.problems;
        },
        get count() {
            return state.problems.length;
        },
        get drawerOpen() {
            return state.drawerOpen;
        },
        set drawerOpen(open: boolean) {
            state.drawerOpen = open;
        },
        requestFocus,
    },
}));

const {default: ProblemsDrawer} = await import("./ProblemsDrawer.svelte");

const txn = (index: number, date: string, description: string): Transaction =>
    ({index, date, description, status: "cleared", postings: [], tags: [], comment: ""}) as unknown as Transaction;

// The engine's real sentence, both halves (see `journal_to_tag_diagnostics` in
// wire.rs): the accepted codes, then what ignoring the tag COST. The second
// clause is why this is a usable warning rather than a shrug, and it is per-tag
// — `valuation:` names a number that will read zero — so a fixture that stops at
// the semicolon is testing a message the engine does not send.
const unanchored: Problem = {
    txnIndex: null,
    account: "assets:property:house",
    rule: "account-tag",
    severity: "warning",
    message:
        "account 'assets:property:house' declares `holdings: real-estate`, which is not one of stocks, other, none; " +
        "the tag is ignored and the account is classified mechanically (does it hold a non-currency commodity?)",
};

const anchored: Problem = {
    txnIndex: 7,
    rule: "unbalanced",
    severity: "error",
    message: "This transaction is unbalanced.",
};

/** Render the drawer OPEN — the only state in which it has rows to assert on. */
function show(problems: Problem[], txns: Transaction[] = []): void {
    state.problems = problems;
    state.txns = txns;
    state.drawerOpen = true;
    render(ProblemsDrawer);
}

describe("COMPONENT ProblemsDrawer — unanchored findings", () => {
    it("renders an account-anchored finding with the ACCOUNT NAME and no click target", () => {
        show([unanchored]);

        // The account name takes the slot a transaction's date would occupy, so
        // the row still answers "what is this about?".
        expect(screen.getByText("assets:property:house")).toBeTruthy();
        expect(screen.getByText(unanchored.message)).toBeTruthy();

        // ...and there is nothing to click. Not a disabled button, not a button
        // with a no-op handler — no button at all.
        expect(screen.queryByRole("button")).toBeNull();
    });

    it("still renders an anchored finding as a button that can jump", () => {
        show([anchored], [txn(7, "2026-03-04", "Rent")]);

        const button = screen.getByRole("button");
        expect(button.textContent).toContain("2026-03-04");
        expect(button.textContent).toContain("Rent");
    });

    it("renders both kinds together, with exactly one button between them", () => {
        show([anchored, unanchored], [txn(7, "2026-03-04", "Rent")]);

        expect(screen.getAllByRole("button")).toHaveLength(1);
        expect(screen.getByText("assets:property:house")).toBeTruthy();
        // Two rules, so two groups, and the badge counts both findings.
        expect(screen.getByText("2 findings")).toBeTruthy();
    });

    it("gives the rule a human label rather than showing the wire id", () => {
        show([unanchored]);
        expect(screen.getByText("Unknown account tag value")).toBeTruthy();
        expect(screen.queryByText("account-tag")).toBeNull();
    });

    it("counts an unanchored finding in the badge, so it cannot be silently ignored", () => {
        show([unanchored]);
        expect(screen.getByText("1 finding")).toBeTruthy();
    });
});

// The drawer is mounted globally by `+layout.svelte`, on every page, and daisyUI
// hides a closed one with `visibility: hidden` — the subtree is still built and
// laid out. So "closed" has to mean "empty", not "invisible": a journal with
// 21,429 findings was otherwise paying ~215,000 DOM nodes on the Reports tab.
//
// jsdom has no layout engine and cannot assert on visibility, which is exactly
// why this asserts on the ABSENCE of the rows instead — the property that
// actually saves the work.
describe("COMPONENT ProblemsDrawer — closed", () => {
    it("renders no findings at all while the drawer is closed", () => {
        state.problems = [anchored, unanchored];
        state.txns = [txn(7, "2026-03-04", "Rent")];
        state.drawerOpen = false;
        render(ProblemsDrawer);

        expect(screen.queryByText("2 findings")).toBeNull();
        expect(screen.queryByRole("button")).toBeNull();
        expect(screen.queryByText("assets:property:house")).toBeNull();
        expect(screen.queryByText(anchored.message)).toBeNull();
    });

    it("still mounts the aside daisyUI slides in, so the open transition survives", () => {
        state.problems = [anchored];
        state.txns = [];
        state.drawerOpen = false;
        const {container} = render(ProblemsDrawer);

        // The panel element itself is unconditional; only its CONTENTS are guarded.
        expect(container.querySelector("aside")).not.toBeNull();
        expect(container.querySelector(".drawer-overlay")).not.toBeNull();
    });
});
