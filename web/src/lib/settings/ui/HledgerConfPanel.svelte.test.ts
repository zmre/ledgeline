// Mounting Settings › hledger config: the seeding effect is latched on an
// object, the shape `routes/effectLatch.test.ts` exists for — only mounting
// can show it does not feed itself.

import {hledgerConfStore} from "#lib/settings/hledgerConfStore.svelte.js";
import {settings} from "#lib/stores/settings.svelte.js";
import {connectFakeEngine, FAKE_ENGINE} from "#lib/testing/fakeEngine.js";
import {hledgerConfBody} from "#lib/testing/hledgerConfFixture.js";
import {fireEvent, render, screen} from "@testing-library/svelte";
import {flushSync} from "svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import HledgerConfPanel from "./HledgerConfPanel.svelte";

async function mountWith(body: Record<string, unknown>): Promise<void> {
    await connectFakeEngine({"/api/hledger-conf": body});
    // A fresh key per test, so `ensure` reads this test's body.
    await hledgerConfStore.ensure(FAKE_ENGINE, `${settings.serverNonce}|${Math.random()}`);
    render(HledgerConfPanel);
    flushSync();
}

afterEach(() => vi.unstubAllGlobals());

describe("COMPONENT HledgerConfPanel", () => {
    it("mounts without a self-feeding effect and shows the file in force", async () => {
        await mountWith(hledgerConfBody({handle: "~/.hledger.conf", origin: "home"}));
        expect(screen.getByTestId("settings-hledger-conf-handle").textContent).toContain("~/.hledger.conf");
        expect((screen.getByTestId("settings-hledger-conf-auto") as HTMLInputElement).checked).toBe(false);
        expect(screen.getByLabelText("Every report (general section) depth")).toHaveProperty("value", "3");
        expect(screen.getAllByTestId("settings-hledger-conf-override")).toHaveLength(1);
    });

    it("says where a first save will create the file", async () => {
        await mountWith(hledgerConfBody({handle: null, origin: null, createHandle: "hledger.conf", revision: ""}));
        expect(screen.getByTestId("settings-hledger-conf-handle").textContent).toContain("saving will create hledger.conf");
    });

    it("shows the hijack sentence and the alias-section warning, and disables every control", async () => {
        await mountWith(
            hledgerConfBody({
                editable: false,
                hijacked: "This config file starts with `print` …",
                readOnlyReason: "This config file starts with `print` …",
                warnings: [{code: "aliasSection", message: "hledger ignores the `[bs]` section"}],
            })
        );
        expect(screen.getByTestId("settings-hledger-conf-hijacked").textContent).toContain("`print`");
        expect(screen.getByTestId("settings-hledger-conf-warnings").textContent).toContain("[bs]");
        expect((screen.getByTestId("settings-hledger-conf-auto") as HTMLInputElement).disabled).toBe(true);
    });

    it("marks an edit unsaved and saves only what changed", async () => {
        await mountWith(hledgerConfBody());
        const sent: unknown[] = [];
        const saved = hledgerConfBody({auto: true, revision: "rev-2"});
        vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
            if (init?.method === "PUT") sent.push(JSON.parse(String(init.body)));
            return Promise.resolve(
                new Response(JSON.stringify(String(input).endsWith("/api/hledger-conf") ? saved : {}), {
                    status: 200,
                    headers: {"Content-Type": "application/json"},
                })
            );
        });

        await fireEvent.click(screen.getByTestId("settings-hledger-conf-auto"));
        expect(screen.getByTestId("settings-hledger-conf-dirty")).toBeDefined();
        await fireEvent.click(screen.getByTestId("settings-hledger-conf-save"));
        await vi.waitFor(() => expect(screen.getByTestId("settings-hledger-conf-saved")).toBeDefined());
        expect(sent).toEqual([{revision: "rev-1", auto: true}]);
    });

    it("refuses an unwritable pattern before a round trip", async () => {
        await mountWith(hledgerConfBody());
        const pattern = screen.getByLabelText("Account pattern");
        await fireEvent.input(pattern, {target: {value: "has space"}});
        await fireEvent.click(screen.getByTestId("settings-hledger-conf-save"));
        expect(screen.getByTestId("settings-hledger-conf-client-errors").textContent).toContain("contains a space");
    });
});
