// Mounting the dry run — the screen the "two alert boxes" report was made
// against.
//
// `AliasEffectPanel.svelte.test.ts` proves the combined panel is one box. This
// proves the DRY RUN shows one, which is a different claim and the one the user
// made: the two notices were siblings HERE, and a merge that left the old block
// behind in this file would pass over there and fail in front of a person.
// Counting alerts on the assembled screen is the only place that shows up.
//
// Props, not a store: this panel takes its dry run as a prop and the resource
// behind it is already driven end to end in `importStore.test.ts`.

import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {dryRun, RENAMES_AND_PARITY, RENAMES_ONLY, ROTH_ALIAS} from "$lib/testing/aliasFixtures";
import type {DryRunResult} from "../importTypes";
import DryRunPanel from "./DryRunPanel.svelte";

const mount = (result: DryRunResult) =>
    render(DryRunPanel, {
        view: "data" as const,
        result,
        error: null,
        aliases: [ROTH_ALIAS],
        writing: false,
        editable: true,
        confWriting: false,
        confWritten: null,
        confError: null,
        onRetry: () => {},
        onWrite: () => {},
        onInstallConf: () => {},
    });

/** Every alert on screen that is talking about aliases. */
const aliasAlerts = (container: HTMLElement): Element[] =>
    [...container.querySelectorAll(".alert")].filter((alert) => (alert.textContent ?? "").toLowerCase().includes("alias"));

describe("COMPONENT DryRunPanel — the alias notices", () => {
    it("shows one alias box, not two", () => {
        const {container} = mount(dryRun(RENAMES_AND_PARITY));

        expect(aliasAlerts(container)).toHaveLength(1);
    });

    it("keeps the command-line notice inside that one box", () => {
        // The merge, seen from the screen: both `data-testid`s still exist —
        // nothing that referenced either has been stranded — and the second is
        // nested in the first rather than beside it.
        mount(dryRun(RENAMES_AND_PARITY));

        expect(screen.getByTestId("imports-alias-effect").contains(screen.getByTestId("imports-cli-parity"))).toBe(true);
    });

    it("shows one box for the rewrite alone when a terminal would agree", () => {
        const {container} = mount(dryRun(RENAMES_ONLY));

        expect(aliasAlerts(container)).toHaveLength(1);
        expect(screen.queryByTestId("imports-cli-parity")).toBeNull();
    });

    it("shows none at all on an import no alias touched", () => {
        const {container} = mount(dryRun(null));

        expect(aliasAlerts(container)).toHaveLength(0);
        // …and the rest of the dry run is still there, so "no alias box" is not
        // being satisfied by a panel that failed to render.
        expect(screen.getByTestId("imports-dry-run-entries")).toBeDefined();
        expect(screen.getByTestId("imports-write-changes")).toBeDefined();
    });
});

describe("COMPONENT DryRunPanel — the equivalent command line", () => {
    it("shows the engine's command VERBATIM", () => {
        const run = dryRun(null);
        mount(run);

        // Byte for byte. The engine builds this with the same argv builder
        // `ledgeline import` is parsed into (WP-16 Phase 3), so any re-quoting,
        // splitting or prettifying here would break the one property the whole
        // feature rests on.
        if (!run.ok) throw new Error("unreachable");
        expect(screen.getByTestId("imports-cli-command").textContent).toBe(run.cliCommand);
    });

    it("copies it to the clipboard and says so", async () => {
        // jsdom has no `navigator.clipboard`, and the component swallows the
        // failure on purpose (an insecure context must not raise an error over a
        // working screen) — so an unstubbed run would silently never latch.
        const writeText = vi.fn().mockResolvedValue(undefined);
        vi.stubGlobal("navigator", {...navigator, clipboard: {writeText}});

        const run = dryRun(null);
        mount(run);
        const button = screen.getByTestId("imports-copy-cli-command");
        expect(button.textContent?.trim()).toBe("Copy");

        await fireEvent.click(button);

        if (!run.ok) throw new Error("unreachable");
        expect(writeText).toHaveBeenCalledWith(run.cliCommand);
        expect(button.textContent?.trim()).toBe("Copied!");
        vi.unstubAllGlobals();
    });

    it("is not offered when hledger refused the import", () => {
        // There is no import to reproduce, so there is no command to copy.
        mount({ok: false, stderr: "hledger: Error: could not parse date\n"});

        expect(screen.queryByTestId("imports-cli-command")).toBeNull();
        expect(screen.queryByTestId("imports-copy-cli-command")).toBeNull();
    });
});

describe("COMPONENT DryRunPanel — rows matched by id (WP-16 Phase 4)", () => {
    it("shows nothing when the rules file declares no id", () => {
        // The overwhelming common case: every rules file written before this
        // feature existed. `idMatches` decodes to null and nothing renders.
        mount(dryRun(null));

        expect(screen.queryByTestId("imports-id-matches")).toBeNull();
    });

    it("shows nothing when there is an id column but nothing worth reporting", () => {
        const run = dryRun(null);
        if (!run.ok) throw new Error("unreachable");
        mount({...run, idMatches: {new: 4, unchanged: 6, statusChanged: [], statusChangedTotal: 0, conflicting: [], conflictingTotal: 0}});

        expect(screen.queryByTestId("imports-id-matches")).toBeNull();
    });

    it("reports a status sync without alarm", () => {
        const run = dryRun(null);
        if (!run.ok) throw new Error("unreachable");
        mount({
            ...run,
            idMatches: {
                new: 0,
                unchanged: 0,
                statusChanged: [{id: "FIT0001", from: "pending", to: "cleared", applied: false}],
                statusChangedTotal: 1,
                conflicting: [],
                conflictingTotal: 0,
            },
        });

        const box = screen.getByTestId("imports-id-matches");
        expect(box.className).toContain("alert-info");
        expect(box.className).not.toContain("alert-warning");
        expect(screen.getByTestId("imports-status-changed").textContent).toContain("pending → cleared");
        expect(screen.queryByTestId("imports-conflicting")).toBeNull();
    });

    it("warns — not just reports — a conflict, and names it, without touching the transaction count", () => {
        // This is the requirement the feature exists to satisfy: warn that a
        // row changed and how, because it is more likely the user changed it on
        // purpose than that the bank's data did — never silently reimport or
        // overwrite it.
        const run = dryRun(null);
        if (!run.ok) throw new Error("unreachable");
        mount({
            ...run,
            idMatches: {
                new: 0,
                unchanged: 0,
                statusChanged: [],
                statusChangedTotal: 0,
                conflicting: [{id: "FIT0002", diffs: [{field: "amount", existing: "-35.60", incoming: "-32.10"}]}],
                conflictingTotal: 1,
            },
        });

        const box = screen.getByTestId("imports-id-matches");
        expect(box.className).toContain("alert-warning");
        const conflicts = screen.getByTestId("imports-conflicting");
        expect(conflicts.textContent).toContain("FIT0002");
        expect(conflicts.textContent).toContain("-35.60");
        expect(conflicts.textContent).toContain("-32.10");
        // The engine already excluded conflicting rows from `count`/`entries` —
        // this panel only ever reports that fact, never recomputes it.
        expect(screen.getByTestId("imports-dry-run-status").textContent).toBe(run.status);
    });
});
