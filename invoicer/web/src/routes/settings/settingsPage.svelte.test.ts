// The Settings page, mounted (SETTINGS-PLAN §4, §6): Save and Discard only
// while something is unsaved, once each; ⌘/Ctrl+S; the confirmation listing
// each effect; the conflict notice after an outside edit; the read-only
// states; the accrual fold; the disabled "coming later" choices; the tabs and
// the shared Appearance panel.
import {fireEvent, render, screen, waitFor, within} from "@testing-library/svelte";
import {tick} from "svelte";
import {beforeEach, describe, expect, it, vi} from "vitest";

const nav = vi.hoisted(() => ({url: new URL("http://localhost/app/settings"), goto: vi.fn()}));
vi.mock("$app/state", () => ({
    page: {
        get url() {
            return nav.url;
        },
        params: {},
    },
}));
vi.mock("$app/navigation", () => ({goto: nav.goto, beforeNavigate: vi.fn()}));
vi.mock("$app/paths", () => ({resolve: (path: string) => `/app${path}`}));

const api = vi.hoisted(() => ({fetch: vi.fn(), check: vi.fn(), save: vi.fn()}));
vi.mock("#lib/settingsApi.js", async (original) => ({
    ...(await original<object>()),
    fetchSettings: api.fetch,
    checkSettings: api.check,
    saveSettings: api.save,
}));

const prefsApi = vi.hoisted(() => ({
    storage: "files" as "files" | "browser",
    patch: vi.fn(),
    pin: vi.fn(),
}));
vi.mock("#lib/uiPrefs.svelte.js", async (original) => {
    const real = await original<typeof import("#lib/uiPrefs.svelte.js")>();
    const answer = () => ({prefs: {theme: "default", uiScale: 100}, storage: prefsApi.storage, localKeys: []});
    const uiPrefs = new real.UiPrefs(
        {
            load: () => Promise.resolve(answer()),
            patch: (values) => {
                prefsApi.patch(values);
                return Promise.resolve(answer());
            },
            pin: (key, pinned) => {
                prefsApi.pin(key, pinned);
                return Promise.resolve({...answer(), localKeys: pinned ? [key] : []});
            },
            beacon: vi.fn(),
        },
        0
    );
    return {...real, uiPrefs};
});

import {noteChange} from "#lib/changes.svelte.js";
import {EditorApiError} from "#lib/editorApi.js";
import {listenForKeys, press} from "#lib/testing/keys.js";
import {settingsPage} from "#lib/testing/settingsFixtures.js";
import {uiPrefs} from "#lib/uiPrefs.svelte.js";
import SettingsPage from "./+page.svelte";

const CLEAN = {problems: [], effects: [], refused: null};

async function mount(): Promise<void> {
    render(SettingsPage);
    await waitFor(() => expect(screen.getByTestId("settings-books")).toBeTruthy());
}

async function edit(testid: string, value: string): Promise<void> {
    await fireEvent.input(screen.getByTestId(testid), {target: {value}});
    await tick();
}

describe("COMPONENT the Settings page", () => {
    listenForKeys();
    beforeEach(async () => {
        nav.url = new URL("http://localhost/app/settings");
        nav.goto.mockReset();
        api.fetch.mockReset().mockResolvedValue(settingsPage());
        api.check.mockReset().mockResolvedValue(CLEAN);
        api.save.mockReset().mockResolvedValue({revision: "rev-2", values: settingsPage().values, set: [], created: false});
        prefsApi.storage = "files";
        prefsApi.patch.mockReset();
        prefsApi.pin.mockReset();
        await uiPrefs.load();
    });

    it("shows the sections in order, with labels and badges, and never a setting's name", async () => {
        await mount();
        const titles = screen.getAllByRole("heading", {level: 2}).map((h) => h.textContent?.trim());
        expect(titles).toEqual(["Company", "New invoices", "Accounts", "Payments", "Editing", "Dashboard", "Files and folders", "Books"]);
        expect(screen.getByTestId("settings-wording").textContent).toContain("Wording");
        expect(screen.getByTestId("setting-dso-days-default").textContent).toBe("Default");
        expect(screen.getByTestId("setting-default-terms-use-default")).toBeTruthy();
        expect(screen.getByTestId("setting-revenue-account-found").textContent).toBe("Found for you: revenue:consulting");
        expect(screen.getByTestId("setting-cash-deposits-affects").textContent).toBe("Affects new bookings");
        expect(screen.getByTestId("filename-example").textContent).toMatch(/^\d{4}-\d{2}-\d{2} Acme Invoice 208\.md$/);
        expect(screen.getByTestId("next-number").textContent).toContain("208");
        const text = screen.getByTestId("settings-page").textContent ?? "";
        for (const leak of ["[[", "{", "invoices-dir", "settlementLeadDays", "ledgeline.toml", "words."]) expect(text).not.toContain(leak);
    });

    it("shows Save and Discard changes once each, only while something is unsaved", async () => {
        await mount();
        expect(screen.queryByTestId("settings-save")).toBeNull();
        expect(screen.queryByTestId("settings-discard")).toBeNull();
        await edit("input-dso-days", "60");
        expect(screen.getAllByRole("button", {name: "Save"})).toHaveLength(1);
        expect(screen.getAllByRole("button", {name: "Discard changes"})).toHaveLength(1);
        expect(screen.getByTestId("settings-rail-dashboard").textContent).toContain("unsaved changes");
        await fireEvent.click(screen.getByTestId("settings-discard"));
        expect(screen.queryByTestId("settings-save")).toBeNull();
        expect((screen.getByTestId("input-dso-days") as HTMLInputElement).value).toBe("90");
    });

    it("saves with ⌘/Ctrl+S, from a field too", async () => {
        await mount();
        await edit("input-dso-days", "60");
        await press("s", screen.getByTestId("input-dso-days"), {ctrlKey: true});
        await waitFor(() => expect(api.save).toHaveBeenCalled());
        expect(api.save).toHaveBeenCalledWith(expect.objectContaining({set: {"dso-days": 60}}));
        await waitFor(() => expect(screen.getByTestId("settings-saved")).toBeTruthy());
    });

    it("confirms a booking change, listing each effect in words", async () => {
        await mount();
        api.check.mockResolvedValueOnce({
            ...CLEAN,
            effects: [{key: "payment-allocation", message: "Payments you apply from now on are split pro rata."}],
        });
        await fireEvent.click(screen.getByTestId("choice-payment-allocation-pro-rata"));
        await fireEvent.click(screen.getByTestId("settings-save"));
        const dialog = await screen.findByTestId("settings-confirm");
        expect(within(dialog).getByTestId("settings-effects").textContent).toContain(
            "Splitting a payment over rows: Payments you apply from now on are split pro rata."
        );
        expect(api.save).not.toHaveBeenCalled();
        await fireEvent.click(within(dialog).getByTestId("settings-confirm-confirm"));
        await waitFor(() => expect(api.save).toHaveBeenCalledWith(expect.objectContaining({confirmed: ["payment-allocation"]})));
    });

    it("lists the waiting deposits when the change is refused", async () => {
        await mount();
        api.check.mockResolvedValueOnce({
            ...CLEAN,
            refused: {
                key: "cash-deposits",
                message: "1 deposit is waiting for its final invoice.",
                deposits: [{number: "205", customer: "Contoso", date: "2026-10-01", waiting: "$1,500.00", bookedTo: []}],
            },
        });
        await fireEvent.click(screen.getByTestId("choice-cash-deposits-liability-until-final"));
        await fireEvent.click(screen.getByTestId("settings-save"));
        const dialog = await screen.findByTestId("settings-refused");
        expect(within(dialog).getByTestId("settings-refused-deposits").textContent).toContain("Deposit invoice 205 for Contoso");
        await fireEvent.click(within(dialog).getByTestId("settings-refused-confirm"));
        expect(screen.queryByTestId("settings-save")).toBeNull();
    });

    it("asks before creating a settings file", async () => {
        api.fetch.mockResolvedValue(settingsPage({file: "missing", set: [], revision: null}));
        await mount();
        expect(screen.getByTestId("settings-no-file")).toBeTruthy();
        await edit("input-dso-days", "60");
        await fireEvent.click(screen.getByTestId("settings-save"));
        const dialog = await screen.findByTestId("settings-create-file");
        expect(dialog.textContent).toContain("These books have no settings file yet. Create it?");
        await fireEvent.click(within(dialog).getByTestId("settings-create-file-confirm"));
        await waitFor(() => expect(api.save).toHaveBeenCalledWith(expect.objectContaining({createFile: true})));
    });

    it("shows the conflict notice when the file changes under unsaved edits", async () => {
        await mount();
        await edit("input-dso-days", "60");
        api.fetch.mockResolvedValueOnce(settingsPage({revision: "rev-9"}));
        noteChange(["ledgeline.toml"]);
        const notice = await screen.findByTestId("conflict-notice");
        expect(notice.textContent).toContain("This settings file changed");
        expect((screen.getByTestId("input-dso-days") as HTMLInputElement).value).toBe("60");
        api.fetch.mockResolvedValueOnce(settingsPage({revision: "rev-9"}));
        await fireEvent.click(screen.getByTestId("conflict-reload"));
        await waitFor(() => expect(screen.queryByTestId("conflict-notice")).toBeNull());
        expect((screen.getByTestId("input-dso-days") as HTMLInputElement).value).toBe("90");
    });

    it("is read-only over a file it can't write, saying why in plain words", async () => {
        api.fetch.mockResolvedValue(
            settingsPage({
                readOnly: true,
                problem: "The invoicer settings in ledgeline.toml have a problem, so they can't be saved here.",
                problems: [{key: "filename", message: "The new invoice file name must include the invoice number."}],
            })
        );
        await mount();
        const alert = screen.getByTestId("settings-read-only");
        expect(alert.textContent).toContain("in the settings file have a problem");
        expect(alert.textContent).toContain("New invoice file name: The new invoice file name must include the invoice number.");
        expect(alert.textContent).not.toContain("ledgeline.toml");
        expect(within(alert).getByRole("button", {name: "Open the settings file"})).toBeTruthy();
        expect((screen.getByTestId("input-dso-days") as HTMLInputElement).disabled).toBe(true);
        expect(screen.queryByTestId("setting-default-terms-use-default")).toBeNull();
    });

    it("folds the cash-books choices in accrual books", async () => {
        api.fetch.mockResolvedValue(settingsPage({books: {basis: "accrual", closedThrough: null}}));
        await mount();
        expect(screen.getByTestId("settings-accrual-note").textContent).toContain("These books use accrual; the cash-books choices don't apply.");
        expect(screen.queryByTestId("setting-cash-deposits")).toBeNull();
        expect(screen.getByTestId("setting-settlementLeadDays")).toBeTruthy();
    });

    it("shows the coming-later choices disabled", async () => {
        await mount();
        for (const id of ["oldest-row-first", "keep-deferral", "first-open-day", "reverse-and-rebook"]) {
            expect((screen.getByTestId(`coming-${id}`) as HTMLInputElement).disabled, id).toBe(true);
        }
    });

    it("edits the tolerance as a percentage and saves basis points; pins a matching setting", async () => {
        await mount();
        const tolerance = screen.getByTestId("input-settlementToleranceBps") as HTMLInputElement;
        expect(tolerance.value).toBe("3");
        await fireEvent.change(tolerance, {target: {value: "2.5"}});
        await fireEvent.click(screen.getByTestId("settings-save"));
        await waitFor(() => expect(prefsApi.patch).toHaveBeenCalledWith({settlementToleranceBps: 250}));
        expect(api.save).not.toHaveBeenCalled();

        await fireEvent.click(screen.getByTestId("pin-settlementLeadDays"));
        await waitFor(() => expect(prefsApi.pin).toHaveBeenCalledWith("settlementLeadDays", true));
    });

    it("turns the file name's chips into the setting", async () => {
        await mount();
        await fireEvent.click(screen.getByTestId("filename-remove-customer"));
        await tick();
        expect(screen.getByTestId("filename-example").textContent).toMatch(/^\d{4}-\d{2}-\d{2} {2}Invoice 208\.md$/);
        await fireEvent.click(screen.getByTestId("settings-save"));
        await waitFor(() => expect(api.save).toHaveBeenCalledWith(expect.objectContaining({set: {filename: "{date}  Invoice {invoice_num}.md"}})));
    });

    it("moves between the tabs with the arrow keys", async () => {
        await mount();
        const books = screen.getByTestId("settings-tab-books");
        expect(books.getAttribute("aria-selected")).toBe("true");
        await fireEvent.keyDown(books, {key: "ArrowRight"});
        expect(nav.goto).toHaveBeenCalledWith("/app/settings?tab=appearance", {replace: true, reset: false});
    });

    it("shows the shared Appearance panel, with the books' pin only where it can be kept", async () => {
        nav.url = new URL("http://localhost/app/settings?tab=appearance");
        render(SettingsPage);
        expect(screen.getByTestId("appearance-shared").textContent).toContain("Shared with Ledgeline");
        expect(screen.getByTestId("appearance-pin-theme").closest("label")?.textContent).toContain("Use for these books only");
        expect(screen.queryByTestId("settings-save")).toBeNull();
    });

    it("hides the pin in browser mode", async () => {
        prefsApi.storage = "browser";
        await uiPrefs.load();
        nav.url = new URL("http://localhost/app/settings?tab=appearance");
        render(SettingsPage);
        expect(screen.queryByTestId("appearance-pin-theme")).toBeNull();
    });

    it("a save over a newer file shows the conflict notice", async () => {
        await mount();
        api.save.mockRejectedValueOnce(new EditorApiError(409, "changed", "The settings file changed since this page read it.", "rev-7"));
        await edit("input-dso-days", "60");
        await fireEvent.click(screen.getByTestId("settings-save"));
        expect(await screen.findByTestId("conflict-notice")).toBeTruthy();
    });
});
