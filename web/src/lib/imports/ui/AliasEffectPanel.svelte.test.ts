// Mounting the combined alias panel.
//
// Three defects reached the user through this markup and all three are mount
// claims, which is why they are here rather than in `aliasModel.test.ts` — every
// function this panel calls was already green while the screen was wrong.
//
//  1. TWO alerts where there is one subject. "There are two alert boxes that
//     come up (both saying much the same thing about aliases -- could they be
//     combined?)" The pure functions cannot see how many boxes their strings
//     ended up in; a mount can count them.
//  2. The columnar layout. `.alert` is `display:grid; grid-auto-flow:column` in
//     daisyUI v5, so the `flex-col` these alerts carried was inert and every
//     child became a narrow column. jsdom has no layout engine and cannot see
//     that — so the class that carries the fix is asserted here, and
//     `routes/alertStacking.test.ts` enforces the same rule over every `.svelte`
//     file at once.
//  3. Staying quiet. Both halves are null on the ordinary import, and a panel
//     that renders an empty box on every dry run is worse than either bug above.
//
// # Why props and not a store
//
// `AliasPanel.svelte.test.ts` drives the real store through a stubbed `fetch`,
// because what broke there was what the panel DID with what the store gave it.
// This panel reads no store: its inputs are its props, so props are what a test
// hands it. Wiring a fetch stub to arrive back at the same object would test the
// decoder twice and this component no better.

import {render, screen, within} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {ALIASES_QUIET, PARITY_BEYOND_RENAMES, RENAMES_AND_PARITY, RENAMES_ONLY, ROTH_ALIAS} from "$lib/testing/aliasFixtures";
import type {AliasEffect} from "../importTypes";
import AliasEffectPanel from "./AliasEffectPanel.svelte";

const mount = (effect: AliasEffect | null, overrides: Record<string, unknown> = {}) =>
    render(AliasEffectPanel, {
        effect,
        aliases: [ROTH_ALIAS],
        editable: true,
        confWriting: false,
        confWritten: null,
        confError: null,
        onInstallConf: () => {},
        ...overrides,
    });

describe("COMPONENT AliasEffectPanel — one panel, not two", () => {
    it("says both facts inside a single alert", () => {
        const {container} = mount(RENAMES_AND_PARITY);

        expect(container.querySelectorAll(".alert")).toHaveLength(1);
        expect(screen.getByText("Your journal's aliases rewrite 2 account names in this import.")).toBeDefined();
        expect(screen.getByText("Run from the command line, this same import would file one account differently.")).toBeDefined();
    });

    it("subordinates the command-line fact inside the rewrite one", () => {
        // Not merely "both are on screen": the parity block is a child of the
        // rewrite panel, which is the structural form of "this is a caveat about
        // that" and the thing two sibling alerts could not express.
        mount(RENAMES_AND_PARITY);

        expect(screen.getByTestId("imports-alias-effect").contains(screen.getByTestId("imports-cli-parity"))).toBe(true);
    });

    it("does not print the same list of accounts twice", () => {
        // The literal complaint. With no hledger.conf in force the parity
        // differences ARE the renames — the fixture's single difference is one
        // of the two renames, character for character — so the old screen
        // printed one list under two headlines in two boxes. Here the second
        // list is replaced by a sentence pointing at the first.
        mount(RENAMES_AND_PARITY);
        const panel = screen.getByTestId("imports-alias-effect");

        expect(within(panel).getByTestId("imports-cli-parity-same")).toBeDefined();
        expect(within(panel).queryByTestId("imports-cli-parity-differences")).toBeNull();
        // Each pair appears exactly once on the whole panel.
        expect(screen.getAllByText("PW Roth IRA - 3077:cash → assets:morganstanley:pw-roth-ira:cash")).toHaveLength(1);
    });

    it("prints the list when a config file makes the two genuinely differ", () => {
        // The other half. Suppressing a list that says something new would be a
        // worse bug than printing one that repeats — this is the case where the
        // terminal lands on an account nothing else on screen mentions, and the
        // lead names the direction, which no reader could infer from pairs.
        mount(PARITY_BEYOND_RENAMES);
        const panel = screen.getByTestId("imports-alias-effect");

        expect(within(panel).getByTestId("imports-cli-parity-differences")).toBeDefined();
        expect(within(panel).queryByTestId("imports-cli-parity-same")).toBeNull();
        expect(panel.textContent).toContain("Each pair is the account a terminal would file it under");
        expect(within(panel).getByTestId("imports-alias-renames")).toBeDefined();
    });
});

describe("COMPONENT AliasEffectPanel — the columnar layout", () => {
    it("stacks with flex, which is what a daisyUI alert needs", () => {
        // jsdom has no layout engine, so this asserts the class that carries the
        // fix rather than the geometry it produces. `.alert` is
        // `display:grid; grid-auto-flow:column`, so `flex-col` on its own is
        // ignored and each child becomes a thin column.
        mount(RENAMES_AND_PARITY);
        const panel = screen.getByTestId("imports-alias-effect");

        expect(panel.classList.contains("flex")).toBe(true);
        expect(panel.classList.contains("flex-col")).toBe(true);
    });

    it("warns only once the two tools disagree", () => {
        // Aliases rewriting account names is what the user asked for by writing
        // them. It is worth seeing before an irreversible step; it is not a
        // problem, and an alert that shouts on every import is one nobody reads
        // when it matters.
        const {container: quiet} = mount(RENAMES_ONLY);
        expect(quiet.querySelector(".alert-info")).not.toBeNull();
        expect(quiet.querySelector(".alert-warning")).toBeNull();

        const {container: loud} = mount(RENAMES_AND_PARITY);
        expect(loud.querySelector(".alert-warning")).not.toBeNull();
    });
});

describe("COMPONENT AliasEffectPanel — when it stays quiet", () => {
    it("renders nothing when no alias is in force", () => {
        const {container} = mount(null);

        expect(container.querySelector(".alert")).toBeNull();
    });

    it("renders nothing when the aliases matched nothing in this statement", () => {
        const {container} = mount(ALIASES_QUIET);

        expect(container.querySelector(".alert")).toBeNull();
    });

    it("shows the rewrite alone when a terminal would agree", () => {
        mount(RENAMES_ONLY);

        expect(screen.getByTestId("imports-alias-effect")).toBeDefined();
        expect(screen.queryByTestId("imports-cli-parity")).toBeNull();
    });
});

describe("COMPONENT AliasEffectPanel — the config fix", () => {
    it("offers the fix with the revision the engine asked to be echoed", () => {
        const onInstallConf = vi.fn();
        mount(RENAMES_AND_PARITY, {onInstallConf});

        screen.getByTestId("imports-cli-parity-fix").click();

        expect(onInstallConf).toHaveBeenCalledWith(RENAMES_AND_PARITY.cli.revision);
    });

    it("shows the exact lines it would add before it is pressed", () => {
        // The conversion widens what matches — a space becomes `.`, which
        // matches any character — so the line is shown, not summarised.
        mount(RENAMES_AND_PARITY);

        expect(screen.getByTestId("imports-cli-parity-additions").textContent).toContain("PW.Roth.IRA.-.3077=assets:morganstanley:pw-roth-ira");
    });

    it("does not offer to write a config file on a read-only server", () => {
        mount(RENAMES_AND_PARITY, {editable: false});

        expect(screen.getByTestId("imports-cli-parity")).toBeDefined();
        expect(screen.queryByTestId("imports-cli-parity-fix")).toBeNull();
    });
});
