// Setting up time tracking: the suggested name (editable, clearable), the
// files it will create asked of the server without writing (again for a new
// name), then Set up; or Use existing…, checked first, then used, with the
// one line to add when the books have no settings file.
import {fireEvent, render, screen} from "@testing-library/svelte";
import {describe, expect, it, vi} from "vitest";
import {fakeApi} from "./fixtures.js";
import SetupView from "./SetupView.svelte";
import type {SetupOffer} from "#lib/time/types.js";

const offer: SetupOffer = {
    suggested_person: "pat",
    file: "time.journal",
    accounts_file: "time-accounts.journal",
    track_file: "time/{person}/{yyyy}-{mm}.timedot",
    found: ["old/hours.timedot"],
};

function mount() {
    const api = fakeApi();
    const onDone = vi.fn();
    render(SetupView, {offer, api, onDone, previewDelayMs: 0});
    return {api, onDone};
}

describe("COMPONENT time tracking's setup", () => {
    it("suggests the name, shows the files it will create, then creates them", async () => {
        const {api, onDone} = mount();
        const person = screen.getByTestId("time-setup-person") as HTMLInputElement;
        expect(person.value).toBe("pat");
        await vi.waitFor(() => expect(screen.getByTestId("time-setup-files").textContent).toContain("time/pat/2026-11.timedot"));
        expect(api.setup).toHaveBeenLastCalledWith("pat", true);
        expect(screen.getByTestId("time-setup-files").textContent).toContain("Gathers all your time");
        // Cleared: no folder per person.
        await fireEvent.input(person, {target: {value: ""}});
        await vi.waitFor(() => expect(screen.getByTestId("time-setup-files").textContent).toContain("time/2026-11.timedot"));
        await fireEvent.click(screen.getByTestId("time-setup-create"));
        await vi.waitFor(() => expect(onDone).toHaveBeenCalled());
        expect(api.setup).toHaveBeenLastCalledWith("", false);
    });

    it("uses an existing file: found ones offered, checked, then used", async () => {
        const {api, onDone} = mount();
        await fireEvent.click(screen.getByTestId("time-setup-existing"));
        expect((screen.getByTestId("time-setup-file") as HTMLInputElement).value).toBe("old/hours.timedot");
        await fireEvent.click(screen.getByTestId("time-setup-check"));
        await vi.waitFor(() => expect(screen.getByTestId("time-setup-checked").textContent).toContain("old/hours.timedot can be used"));
        expect(api.adopt).toHaveBeenLastCalledWith("old/hours.timedot", true);
        await fireEvent.click(screen.getByTestId("time-setup-adopt"));
        await vi.waitFor(() => expect(onDone).toHaveBeenCalled());
        expect(api.adopt).toHaveBeenLastCalledWith("old/hours.timedot", false);
    });

    it("says what to add when there's no settings file to record the choice in", async () => {
        const {api, onDone} = mount();
        api.adopt.mockImplementation(async (file: string) => ({file, format: "journal", needs_setting: true, written: false}));
        await fireEvent.click(screen.getByTestId("time-setup-existing"));
        await fireEvent.click(screen.getByTestId("time-setup-check"));
        await vi.waitFor(() => screen.getByTestId("time-setup-adopt"));
        await fireEvent.click(screen.getByTestId("time-setup-adopt"));
        await vi.waitFor(() => expect(screen.getByTestId("time-setup-setting").textContent).toContain('file = "old/hours.timedot"'));
        expect(onDone).not.toHaveBeenCalled();
    });
});
