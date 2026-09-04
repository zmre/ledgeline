// Two things about the rules list, both of them keyboard-and-mouse behaviour
// that no pure function can hold.
//
// # Reorder
//
// J/K reorder rules from the keyboard, through the same `moveRule` the ↑/↓
// buttons use. The load-bearing assertion is that the cursor FOLLOWS the moved
// rule. Order is semantics in an hledger rules file ("later matches win"), so a
// user is normally moving one rule several places — and a cursor that stayed put
// would mean the second J moved a different rule than the first.
//
// # Display and edit
//
// Every rule is one summary line until it is opened, and exactly one can be open
// at a time. That is the whole redesign: the list used to render every rule as a
// full editor, which made a file of any size unscannable. So the assertions here
// are mostly about what is NOT on screen — no fields for a rule nobody is
// editing — and about the open card surviving, or not surviving, the things that
// happen around it.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {afterEach, describe, expect, it} from "vitest";
import {keymap} from "$lib/keys/keymap.svelte";
import type {FormItem} from "../model";
import RulesList from "./RulesList.svelte";

afterEach(() => keymap.reset());

function rule(pattern: string, account: string): FormItem {
    return {kind: "ifBlock", id: null, groups: [{matchers: [{field: "", pattern}]}], assignments: [{field: "account2", value: account}], control: null};
}

/**
 * Rebuilt per test, and DEEPLY reactive.
 *
 * Rebuilt because the cards bind straight into these objects, so a shared array
 * would leak edits between tests. `$state` because that is how the real panel
 * holds `form.items`, and it is what makes a nested write — the card assigning
 * `rule.groups` — reach the screen. A plain array renders once and then quietly
 * stops agreeing with itself, which is the same trap `AliasPanel` documents.
 */
function items(): FormItem[] {
    const list = $state([rule("ACME PAYROLL", "income:salary"), rule("COFFEE", "expenses:coffee"), rule("SHELL", "expenses:gas")]);
    return list;
}

type Props = {
    items: FormItem[];
    accountNames: string[];
    csvFields: string[];
    fallbackAccount: string;
    savedAt: number | null;
    onChange: (items: FormItem[]) => void;
    disabled: boolean;
};

function props(overrides: Partial<Props> = {}): Props {
    return {
        items: items(),
        accountNames: ["income:salary", "expenses:coffee", "expenses:gas"],
        csvFields: [],
        fallbackAccount: "expenses:misc",
        savedAt: null,
        onChange: () => {},
        disabled: false,
        ...overrides,
    };
}

function mount(overrides: Partial<Props> = {}) {
    return render(RulesList, {props: props(overrides)});
}

/**
 * A list wired to a parent that actually applies what it is handed.
 *
 * Needed wherever the assertion is about what happens AFTER the document
 * changes — a reorder, an insert — because the component does not own `items`
 * and a stub `onChange` leaves the list rendering the array it started with.
 */
function mountLive(overrides: Partial<Props> = {}) {
    // Filled in once the component exists, because the callback that uses it has
    // to be handed to `render` before `render` can return.
    let apply: ((next: Props) => void) | null = null;
    const state: Props = {
        ...props(overrides),
        onChange: (next) => {
            state.items = next;
            apply?.({...state});
        },
    };
    const view = render(RulesList, {props: state});
    apply = (next) => void view.rerender(next);
    return {
        view,
        get items(): FormItem[] {
            return state.items;
        },
        /** What the panel does when a save lands: hand the list a newer `savedAt`. */
        save(at: number): void {
            state.savedAt = at;
            apply?.({...state});
        },
    };
}

async function press(key: string): Promise<void> {
    keymap.handle(new KeyboardEvent("keydown", {key, cancelable: true}));
    await tick();
}

async function click(name: string): Promise<void> {
    screen.getByRole("button", {name}).click();
    await tick();
}

const cursorAt = (): string | null => document.querySelector("[aria-current='true']")?.getAttribute("data-rule") ?? null;
/** The rule whose editor is open, by the position in its heading, or null. */
const openRule = (): string | null => screen.queryByRole("button", {name: /^Close rule /})?.getAttribute("aria-label") ?? null;

describe("COMPONENT RulesList cursor", () => {
    it("has no cursor until a key moves it", () => {
        mount();

        expect(cursorAt()).toBeNull();
    });

    it("moves between cards with j and k", async () => {
        mount();

        await press("j");
        expect(cursorAt()).toBe("0");

        await press("j");
        expect(cursorAt()).toBe("1");

        await press("k");
        expect(cursorAt()).toBe("0");
    });

    it("jumps to the ends with G and gg", async () => {
        mount();

        await press("G");
        expect(cursorAt()).toBe("2");

        await press("g");
        await press("g");
        expect(cursorAt()).toBe("0");
    });

    it("clears on Escape", async () => {
        mount();
        await press("j");

        await press("Escape");

        expect(cursorAt()).toBeNull();
    });
});

describe("COMPONENT RulesList reorder", () => {
    it("moves the rule down on J, through the same moveRule the buttons use", async () => {
        let moved: FormItem[] | null = null;
        mount({onChange: (next) => (moved = next)});
        await press("j");

        await press("J");

        expect(moved).not.toBeNull();
        // ACME was first; after J it is second.
        expect(screen.getByRole("button", {name: "Move rule 2 up"})).toBeDefined();
    });

    it("takes the cursor with the moved rule", async () => {
        // Otherwise the second J moves a DIFFERENT rule than the first, which is
        // how you silently scramble a rules file.
        mount();
        await press("j");
        expect(cursorAt()).toBe("0");

        await press("J");

        expect(cursorAt()).toBe("1");
    });

    it("does nothing at the ends rather than wrapping", async () => {
        // Wrapping a reorder would move a rule from the top of the file to the
        // bottom on one keystroke — a semantic change wearing a cosmetic's
        // clothes, and not what the ↑/↓ buttons do (they disable at the bounds).
        let calls = 0;
        mount({onChange: () => (calls += 1)});
        await press("j");

        await press("K");

        expect(calls).toBe(0);
    });

    it("does not reorder while the panel is disabled", async () => {
        let calls = 0;
        mount({accountNames: [], onChange: () => (calls += 1), disabled: true});
        await press("j");

        await press("J");

        expect(calls).toBe(0);
    });
});

describe("COMPONENT RulesList display and edit", () => {
    it("shows every rule as one summary line, with nothing to type into", () => {
        mount();

        expect(screen.getAllByTestId("imports-rule")).toHaveLength(3);
        expect(screen.getByText("IF row ~ ACME PAYROLL → account2 = income:salary")).toBeDefined();
        // The point of the whole split: a list you can read is a list with no
        // fields in it.
        expect(screen.queryAllByRole("textbox")).toHaveLength(0);
        expect(screen.queryAllByRole("combobox")).toHaveLength(0);
    });

    it("opens one rule when its summary is clicked, and leaves the rest collapsed", async () => {
        mount();

        await click("Edit rule 2");

        expect(openRule()).toBe("Close rule 2");
        expect((screen.getByLabelText("Rule 2, group 1, match 1 text") as HTMLInputElement).value).toBe("COFFEE");
        // One editor, not three: the other two rules are still summary lines.
        expect(screen.getAllByRole("button", {name: /^Edit rule /})).toHaveLength(2);
    });

    it("collapses again on Done, keeping the rule in the list", async () => {
        mount();
        await click("Edit rule 2");

        await click("Close rule 2");

        expect(openRule()).toBeNull();
        expect(screen.getAllByTestId("imports-rule")).toHaveLength(3);
        expect(screen.getByText("IF row ~ COFFEE → account2 = expenses:coffee")).toBeDefined();
    });

    it("opens only one rule at a time", async () => {
        mount();
        await click("Edit rule 1");

        await click("Edit rule 3");

        expect(openRule()).toBe("Close rule 3");
        expect(screen.queryByLabelText("Rule 1, group 1, match 1 text")).toBeNull();
    });

    // Enter opens the thing under the cursor, as it does on every other list in
    // this app. It could not before, because there was no collapsed state to
    // open — the old binding focused a control in an already-expanded card.
    it("opens and closes the cursored rule with Enter", async () => {
        mount();
        await press("j");
        await press("j");

        await press("Enter");
        expect(openRule()).toBe("Close rule 2");

        await press("Enter");
        expect(openRule()).toBeNull();
    });

    it("backs out one step at a time on Escape: the open rule first, then the cursor", async () => {
        mount();
        await press("j");
        await press("Enter");
        expect(openRule()).toBe("Close rule 1");

        await press("Escape");
        expect(openRule()).toBeNull();
        expect(cursorAt()).toBe("0");

        await press("Escape");
        expect(cursorAt()).toBeNull();
    });

    it("adds a rule already open, because a blank summary line says nothing", async () => {
        const live = mountLive();

        await click("+ Add rule");

        expect(live.items).toHaveLength(4);
        expect(openRule()).toBe("Close rule 4");
        expect(screen.getByLabelText("Rule 4, group 1, match 1 text")).toBeDefined();
    });

    // Positions are the only identity these entries have, so the open card has
    // to be carried through the shuffle its own rule goes through — otherwise a
    // reorder silently swaps which rule is being edited.
    it("keeps the same rule open when it is moved", async () => {
        const live = mountLive();
        await click("Edit rule 1");
        expect((screen.getByLabelText("Rule 1, group 1, match 1 text") as HTMLInputElement).value).toBe("ACME PAYROLL");

        await click("Move rule 1 down");

        expect(openRule()).toBe("Close rule 2");
        expect((screen.getByLabelText("Rule 2, group 1, match 1 text") as HTMLInputElement).value).toBe("ACME PAYROLL");
        expect(live.items.map((item) => (item.kind === "ifBlock" ? item.groups[0]?.matchers[0]?.pattern : null))).toEqual(["COFFEE", "ACME PAYROLL", "SHELL"]);
    });

    // A save is the one thing this list cannot see from its own props, so the
    // panel hands it `savedAt`. The edit has landed; leaving the editor open
    // over it invites a second, accidental edit of a finished rule.
    it("closes the open rule when a save lands, and can be opened again after", async () => {
        const live = mountLive();
        await click("Edit rule 2");

        live.save(1000);
        await tick();

        expect(openRule()).toBeNull();

        // Not latched shut: `savedAt` stays set until the next edit, and a list
        // that treated "is set" as "close" could never be opened again.
        await click("Edit rule 2");
        expect(openRule()).toBe("Close rule 2");
    });
});

describe("COMPONENT RulesList AND-groups", () => {
    it("adds a condition to the group being edited, and a new group beside it", async () => {
        const live = mountLive();
        await click("Edit rule 2");

        await click("Add an AND condition to group 1 of rule 2");
        await click("Add an OR group to rule 2");

        expect(screen.getByLabelText("Rule 2, group 1, match 2 text")).toBeDefined();
        expect(screen.getByLabelText("Rule 2, group 2, match 1 text")).toBeDefined();
        const edited = live.items[1];
        expect(edited?.kind === "ifBlock" && edited.groups.map((group) => group.matchers.length)).toEqual([2, 1]);
    });

    it("drops a group once its last condition is removed, because the engine refuses an empty one", async () => {
        const live = mountLive();
        await click("Edit rule 2");
        await click("Add an OR group to rule 2");

        await click("Remove match 1 from group 2 of rule 2");

        expect(screen.queryByLabelText("Rule 2, group 2, match 1 text")).toBeNull();
        const edited = live.items[1];
        expect(edited?.kind === "ifBlock" && edited.groups).toHaveLength(1);
    });

    it("summarizes an AND-group as one line once it is closed", async () => {
        const live = mountLive();
        const edited = live.items[1];
        if (edited?.kind !== "ifBlock") throw new Error("expected a rule");
        edited.groups[0]!.matchers = [...edited.groups[0]!.matchers, {field: "card", pattern: "personal"}];
        await tick();

        expect(screen.getByText("IF row ~ COFFEE AND card ~ personal → account2 = expenses:coffee")).toBeDefined();
    });
});

describe("COMPONENT RulesList skip/end", () => {
    it("puts the picked control on the rule as a word, and 'as usual' back to null", async () => {
        const live = mountLive();
        await click("Edit rule 2");

        const select = screen.getByLabelText("Rule 2 row handling") as HTMLSelectElement;
        // The empty option is the one the model spells `null`. Sending `""` to
        // the engine would be a third control word and a 400, so the round trip
        // through "" and back has to land on null rather than on a blank string.
        expect(select.value).toBe("");

        await fireEvent.change(select, {target: {value: "skip"}});
        const skipped = live.items[1];
        expect(skipped?.kind === "ifBlock" && skipped.control).toBe("skip");

        await fireEvent.change(select, {target: {value: ""}});
        const cleared = live.items[1];
        expect(cleared?.kind === "ifBlock" && cleared.control).toBeNull();
    });

    it("summarizes a skipping rule by what happens to the row", async () => {
        const live = mountLive();
        const edited = live.items[1];
        if (edited?.kind !== "ifBlock") throw new Error("expected a rule");
        edited.control = "skip";
        edited.assignments = [];
        await tick();

        expect(screen.getByText("IF row ~ COFFEE → skip this row")).toBeDefined();
    });
});
