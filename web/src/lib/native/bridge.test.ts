// The page's half of the iOS app bridge (plan 42), in the node project: the
// window, document and history it touches are stubbed per test, so every
// branch — native and not — is a function call.
//
// The message shapes pinned here are the contract the Swift side decodes
// (plans/42-mobile-app.md, "The native bridge contract"): a renamed key
// compiles on both sides and silently empties the app's menu.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {
    addActionProvider,
    addOverlay,
    back,
    GLOBAL_ACTION_ORDER,
    HistoryDepth,
    installNativeBridge,
    lastPageMessage,
    noteNavigation,
    publishPage,
    resetNativeBridge,
    runAction,
    SECTION_IDS,
    sveltekitHistoryIndex,
    type NativeApi,
    type PageMeta,
} from "./bridge.svelte";
import {isNative, MESSAGE_HANDLER, NATIVE_API, NATIVE_CLASS, NATIVE_GLOBAL, nativeInfo, postNative} from "./detect";
import {haptic, share, toBase64} from "./share";

interface FakeWindow {
    __LEDGELINE_NATIVE__?: unknown;
    webkit?: {messageHandlers: Record<string, {postMessage(message: unknown): void}>};
    ledgelineNative?: NativeApi;
}

let posted: unknown[];
let historyBack: ReturnType<typeof vi.fn>;
let win: FakeWindow;

function stubPage({native, handler = true, htmlClass = false}: {native: boolean; handler?: boolean; htmlClass?: boolean}): void {
    posted = [];
    win = {};
    if (native) win.__LEDGELINE_NATIVE__ = {platform: "ios", idiom: "phone", version: 1};
    if (handler) win.webkit = {messageHandlers: {[MESSAGE_HANDLER]: {postMessage: (message: unknown) => posted.push(message)}}};
    vi.stubGlobal("window", win);
    const classes = new Set(htmlClass ? [NATIVE_CLASS] : []);
    vi.stubGlobal("document", {documentElement: {classList: {contains: (name: string) => classes.has(name)}}});
    historyBack = vi.fn();
    vi.stubGlobal("history", {back: historyBack, state: null});
}

const META: PageMeta = {
    title: "Journal",
    sections: SECTION_IDS.map((id) => ({id, label: id[0].toUpperCase() + id.slice(1), active: id === "journal"})),
};

beforeEach(() => {
    stubPage({native: true});
    resetNativeBridge();
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("detection", () => {
    it("is native with the global, or with the class alone", () => {
        expect(isNative()).toBe(true);
        expect(nativeInfo()).toEqual({platform: "ios", idiom: "phone", version: 1});

        stubPage({native: false, htmlClass: true});
        expect(isNative()).toBe(true);
        expect(nativeInfo()).toBeNull();
    });

    it("is not native in a browser, nor with no window at all", () => {
        stubPage({native: false});
        expect(isNative()).toBe(false);
        vi.unstubAllGlobals();
        expect(isNative()).toBe(false);
    });

    it("keeps the names the Swift side uses", () => {
        expect([NATIVE_GLOBAL, NATIVE_CLASS, MESSAGE_HANDLER, NATIVE_API]).toEqual(["__LEDGELINE_NATIVE__", "ll-native", "ledgeline", "ledgelineNative"]);
        expect(SECTION_IDS).toEqual(["journal", "holdings", "budget", "projections", "reports", "settings"]);
    });
});

describe("outside the app, everything is a no-op", () => {
    beforeEach(() => stubPage({native: false}));

    it("posts nothing, installs nothing, and says so", () => {
        expect(postNative({type: "page"})).toBe(false);
        expect(publishPage(META)).toBe(false);
        expect(share("a.xlsx", "application/x", new Uint8Array([1]))).toBe(false);
        expect(haptic("light")).toBe(false);
        const uninstall = installNativeBridge({navigate: () => {}});
        expect(win.ledgelineNative).toBeUndefined();
        uninstall();
        expect(posted).toEqual([]);
    });

    it("posts nothing in the app before its handler is installed", () => {
        stubPage({native: true, handler: false});
        expect(postNative({type: "haptic", style: "light"})).toBe(false);
    });
});

describe("page message", () => {
    it("has exactly the contract's shape", () => {
        addActionProvider(() => [
            {id: "export", label: "Export to Excel", icon: "square.and.arrow.up", run: () => {}},
            {id: "wipe", label: "Wipe", destructive: true, run: () => {}},
        ]);

        expect(publishPage(META)).toBe(true);

        expect(posted).toEqual([
            {
                type: "page",
                title: "Journal",
                sections: [
                    {id: "journal", label: "Journal", active: true},
                    {id: "holdings", label: "Holdings", active: false},
                    {id: "budget", label: "Budget", active: false},
                    {id: "projections", label: "Projections", active: false},
                    {id: "reports", label: "Reports", active: false},
                    {id: "settings", label: "Settings", active: false},
                ],
                // `run` never crosses the bridge; `icon`/`destructive` only when set.
                actions: [
                    {id: "export", label: "Export to Excel", icon: "square.and.arrow.up"},
                    {id: "wipe", label: "Wipe", destructive: true},
                ],
                canGoBack: false,
            },
        ]);
    });

    it("is deduped: the same state twice posts once, a change posts again", () => {
        let label = "Problems";
        addActionProvider(() => [{id: "problems", label, run: () => {}}]);

        expect(publishPage(META)).toBe(true);
        expect(publishPage(META)).toBe(false);
        label = "Problems (3)";
        expect(publishPage(META)).toBe(true);
        expect(posted).toHaveLength(2);
        expect(lastPageMessage()?.actions).toEqual([{id: "problems", label: "Problems (3)"}]);
    });

    it("orders actions by order then registration, global ones last, first of an id wins", () => {
        addActionProvider(() => [{id: "refresh", label: "Refresh everything", run: () => {}}], GLOBAL_ACTION_ORDER);
        addActionProvider(() => [{id: "add-transaction", label: "Add transaction", run: () => {}}]);
        addActionProvider(() => [{id: "export", label: "Export to Excel", run: () => {}}], 20);
        addActionProvider(() => [{id: "export", label: "A second export", run: () => {}}], 20);

        publishPage(META);

        expect(lastPageMessage()?.actions.map((a) => a.label)).toEqual(["Add transaction", "Export to Excel", "Refresh everything"]);
    });

    it("drops an unmounted provider's actions", () => {
        const remove = addActionProvider(() => [{id: "export", label: "Export to Excel", run: () => {}}]);
        remove();
        publishPage(META);
        expect(lastPageMessage()?.actions).toEqual([]);
    });

    it("says canGoBack while an overlay is open or there is in-app history", () => {
        const close = addOverlay(() => {});
        publishPage(META);
        expect(lastPageMessage()?.canGoBack).toBe(true);
        close();
        publishPage(META);
        expect(lastPageMessage()?.canGoBack).toBe(false);

        noteNavigation("enter", undefined, null);
        noteNavigation("link", undefined, null);
        publishPage(META);
        expect(lastPageMessage()?.canGoBack).toBe(true);
    });
});

describe("back()", () => {
    it("closes the topmost overlay first, then goes back in history, then gives up", () => {
        const closed: string[] = [];
        noteNavigation("enter", undefined, null);
        noteNavigation("link", undefined, null);
        const removeLower = addOverlay(() => closed.push("drawer"));
        const removeUpper = addOverlay(() => closed.push("dialog"));

        expect(back()).toBe(true);
        expect(closed).toEqual(["dialog"]);
        expect(historyBack).not.toHaveBeenCalled();

        // The overlay's own close is what unregisters it (dismissible's detach).
        removeUpper();
        expect(back()).toBe(true);
        expect(closed).toEqual(["dialog", "drawer"]);
        removeLower();

        expect(back()).toBe(true);
        expect(historyBack).toHaveBeenCalledTimes(1);
        noteNavigation("popstate", -1, null);

        expect(back()).toBe(false);
        expect(historyBack).toHaveBeenCalledTimes(1);
    });
});

describe("history depth", () => {
    it("follows SvelteKit's history index, so a replace adds nothing", () => {
        const depth = new HistoryDepth();
        expect(depth.note("enter", undefined, 1000)).toBe(0);
        expect(depth.note("link", undefined, 1001)).toBe(1);
        // goto(…, {replace: true}): same index.
        expect(depth.note("goto", undefined, 1001)).toBe(1);
        expect(depth.note("goto", undefined, 1002)).toBe(2);
        expect(depth.note("popstate", -1, 1001)).toBe(1);
        expect(depth.note("popstate", -1, 1000)).toBe(0);
    });

    it("counts navigations when the index is not readable", () => {
        const depth = new HistoryDepth();
        expect(depth.note("enter", undefined, null)).toBe(0);
        expect(depth.note("link", undefined, null)).toBe(1);
        expect(depth.note("goto", undefined, null)).toBe(2);
        expect(depth.note("popstate", -2, null)).toBe(0);
        expect(depth.note("popstate", -1, null)).toBe(0);
    });

    it("reads the index from where SvelteKit 3 keeps it, and nothing else", () => {
        expect(sveltekitHistoryIndex({"sveltekit:metadata": {historyIndex: 1794614867141}})).toBe(1794614867141);
        expect(sveltekitHistoryIndex({"sveltekit:metadata": {}})).toBeNull();
        expect(sveltekitHistoryIndex(null)).toBeNull();
        expect(sveltekitHistoryIndex("nope")).toBeNull();
    });
});

describe("window.ledgelineNative", () => {
    it("navigates to known sections, runs actions, goes back and reports state", () => {
        const went: string[] = [];
        let ran = 0;
        const uninstall = installNativeBridge({navigate: (id) => went.push(id)});
        const api = win.ledgelineNative!;
        addActionProvider(() => [{id: "refresh", label: "Refresh everything", run: () => (ran += 1)}]);

        expect(api.navigate("reports")).toBe(true);
        expect(api.navigate("imports")).toBe(false);
        expect(went).toEqual(["reports"]);

        expect(api.action("refresh")).toBe(true);
        expect(api.action("nope")).toBe(false);
        expect(ran).toBe(1);

        expect(api.back()).toBe(false);

        expect(api.state()).toBeNull();
        publishPage(META);
        expect(api.state()).toEqual(posted[0]);

        uninstall();
        expect(win.ledgelineNative).toBeUndefined();
    });

    it("runAction reads the actions as they are now", () => {
        let ran = "";
        let id = "a";
        addActionProvider(() => [{id, label: id, run: () => (ran = id)}]);
        id = "b";
        expect(runAction("a")).toBe(false);
        expect(runAction("b")).toBe(true);
        expect(ran).toBe("b");
    });
});

describe("share and haptic", () => {
    it("posts the file as base64 with its name and type", () => {
        const bytes = new TextEncoder().encode("PK\u0003\u0004 workbook");
        expect(share("budget.xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", bytes.buffer)).toBe(true);
        expect(posted).toEqual([
            {
                type: "share",
                filename: "budget.xlsx",
                mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                base64: Buffer.from(bytes).toString("base64"),
            },
        ]);
    });

    it("encodes a buffer bigger than one chunk exactly", () => {
        const bytes = Uint8Array.from({length: 100_000}, (_, i) => (i * 31) % 256);
        expect(toBase64(bytes)).toBe(Buffer.from(bytes).toString("base64"));
    });

    it("posts a haptic style", () => {
        expect(haptic("success")).toBe(true);
        expect(posted).toEqual([{type: "haptic", style: "success"}]);
    });
});
