// The contract picker, mounted: search by name, the system's open dialog in
// the window, and its refusal of a file outside the books folder.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import type {FileCandidate, FileList} from "#lib/customers.js";
import ContractPicker from "./ContractPicker.svelte";

const files: FileCandidate[] = [
    {path: "customers/contracts/Customer X MSA 2026.md", name: "Customer X MSA 2026", kind: "contract", entry: "[[Customer X MSA 2026]]"},
    {path: "customers/contracts/SOW 3.pdf", name: "SOW 3.pdf", kind: null, entry: "/customers/contracts/SOW 3.pdf"},
    {path: "notes/Kickoff.md", name: "Kickoff", kind: null, entry: "[[Kickoff]]"},
];
const list =
    (pick: boolean): (() => Promise<FileList>) =>
    () =>
        Promise.resolve({pick, open: pick, files});

describe("COMPONENT ContractPicker", () => {
    it("finds a note or file by name and picks it", async () => {
        const onPick = vi.fn();
        render(ContractPicker, {onPick, onClose: vi.fn(), load: list(false)});
        expect(await screen.findAllByTestId("contract-candidate")).toHaveLength(3);
        // No native dialog in a browser.
        expect(screen.queryByTestId("contract-choose-file")).toBeNull();
        await fireEvent.input(screen.getByTestId("contract-search"), {target: {value: "sow"}});
        const shown = screen.getAllByTestId("contract-candidate");
        expect(shown).toHaveLength(1);
        expect(shown[0].textContent).toContain("SOW 3.pdf");
        expect(shown[0].textContent).toContain("customers/contracts");
        await fireEvent.click(shown[0]);
        expect(onPick).toHaveBeenCalledWith(files[1]);
        await fireEvent.input(screen.getByTestId("contract-search"), {target: {value: "nothing like it"}});
        expect(screen.getByTestId("contract-no-match")).toBeDefined();
    });

    it("chooses a file with the system's dialog, and says why one outside the books is refused", async () => {
        const onPick = vi.fn();
        const pick = vi
            .fn<() => Promise<FileCandidate | null>>()
            .mockResolvedValueOnce(null)
            .mockRejectedValueOnce(new Error("Contract.pdf isn't in the books folder. Contracts are kept with the books."))
            .mockResolvedValueOnce(files[1]);
        render(ContractPicker, {onPick, onClose: vi.fn(), load: list(true), pick});
        const choose = await screen.findByTestId("contract-choose-file");
        await fireEvent.click(choose);
        await vi.waitFor(() => expect(pick).toHaveBeenCalledTimes(1));
        expect(onPick).not.toHaveBeenCalled();
        await fireEvent.click(choose);
        expect((await screen.findByTestId("contract-picker-error")).textContent).toContain("isn't in the books folder");
        await fireEvent.click(choose);
        await vi.waitFor(() => expect(onPick).toHaveBeenCalledWith(files[1]));
    });

    it("closes on Escape and Cancel", async () => {
        const onClose = vi.fn();
        render(ContractPicker, {onPick: vi.fn(), onClose, load: list(false)});
        await fireEvent.keyDown(screen.getByTestId("contract-search"), {key: "Escape"});
        await fireEvent.click(screen.getByRole("button", {name: "Cancel"}));
        expect(onClose).toHaveBeenCalledTimes(2);
    });
});
