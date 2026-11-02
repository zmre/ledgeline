// The `reveal` action and its arm, over real DOM nodes. In the `components`
// project (the `.svelte.test.ts` name is what routes it there) for jsdom's DOM,
// not for any Svelte: the arithmetic is `reveal.test.ts`'s, and here every rect
// is stubbed — jsdom has no layout, so a rect is only ever what a test says.

import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {createRevealArm, reveal, revealTarget} from "./reveal";

/** Every node below the fold of jsdom's 768px window, unless a test says otherwise. */
const FAR = {top: 2000, height: 500};

let scrollBy: ReturnType<typeof vi.fn>;
let rects: Map<Element, {top: number; height: number}>;

function node(parent: HTMLElement = document.body): HTMLElement {
    const element = document.createElement("div");
    parent.appendChild(element);
    return element;
}

/** Let the arm's after-layout flush run. */
const frame = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 30));

beforeEach(() => {
    rects = new Map();
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
        const {top, height} = rects.get(this) ?? FAR;
        return {top, height, bottom: top + height, left: 0, right: 0, width: 0, x: 0, y: top, toJSON: () => ({})} as DOMRect;
    });
    scrollBy = vi.fn();
    vi.stubGlobal("scrollBy", scrollBy);
});

afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
    document.documentElement.style.scrollPaddingTop = "";
});

describe("COMPONENT reveal", () => {
    it("marks the node, so a test can find what is revealed", () => {
        const element = node();
        reveal(element, {arm: null});
        expect(element.dataset.reveal).toBe("");
    });

    it("does nothing on mount while the arm is disarmed — restored content stays put", async () => {
        reveal(node(), {arm: createRevealArm()});
        await frame();
        expect(scrollBy).not.toHaveBeenCalled();
    });

    it("does nothing with no arm at all", async () => {
        reveal(node(), {arm: null});
        await frame();
        expect(scrollBy).not.toHaveBeenCalled();
    });

    it("scrolls a node that mounts below the fold once armed, smoothly", async () => {
        const arm = createRevealArm();
        arm.arm();
        reveal(node(), {arm});
        await frame();
        // 2000 + 40% of 768 - 768, with no header.
        expect(scrollBy).toHaveBeenCalledExactlyOnceWith({top: 1539, behavior: "smooth"});
    });

    it("stops the top at the header the layout publishes as scroll-padding-top", async () => {
        document.documentElement.style.scrollPaddingTop = "64px";
        const arm = createRevealArm();
        arm.arm();
        const element = node();
        rects.set(element, {top: 800, height: 5000});
        reveal(element, {arm});
        await frame();
        // 800 + 307.2 - 768 = 339 is under the 736 ceiling.
        expect(scrollBy).toHaveBeenCalledWith({top: 339, behavior: "smooth"});
    });

    it("scrolls instantly under prefers-reduced-motion", async () => {
        vi.spyOn(window, "matchMedia").mockImplementation((query: string) => ({matches: query.includes("reduce")}) as MediaQueryList);
        const arm = createRevealArm();
        arm.arm();
        reveal(node(), {arm});
        await frame();
        expect(scrollBy).toHaveBeenCalledWith(expect.objectContaining({behavior: "auto"}));
    });

    it("leaves a node whose top is on screen alone", async () => {
        const arm = createRevealArm();
        arm.arm();
        const element = node();
        rects.set(element, {top: 300, height: 900});
        reveal(element, {arm});
        await frame();
        expect(scrollBy).not.toHaveBeenCalled();
    });

    it("re-checks when its key changes, and only then", async () => {
        const arm = createRevealArm();
        const element = node();
        const action = reveal(element, {arm, key: "loading"});
        arm.arm();
        action.update({arm, key: "loading"});
        await frame();
        expect(scrollBy).not.toHaveBeenCalled();

        action.update({arm, key: "data"});
        await frame();
        expect(scrollBy).toHaveBeenCalledOnce();
    });

    it("drops a request for a node removed before the frame", async () => {
        const arm = createRevealArm();
        arm.arm();
        const element = node();
        reveal(element, {arm});
        element.remove();
        await frame();
        expect(scrollBy).not.toHaveBeenCalled();
    });

    it("scrolls once per frame, for the outer section, when nested ones mount together", async () => {
        // The dry run mounting with the review strip inside it: two scrolls
        // measured from one layout would each go their own way.
        const arm = createRevealArm();
        arm.arm();
        const outer = node();
        const inner = node(outer);
        rects.set(outer, {top: 900, height: 2000});
        rects.set(inner, {top: 1500, height: 100});
        reveal(inner, {arm});
        reveal(outer, {arm});
        await frame();
        expect(scrollBy).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({top: Math.round(900 + 0.4 * 768 - 768)}));
    });
});

describe("COMPONENT revealTarget", () => {
    it("picks the first in document order of the outermost nodes", () => {
        const first = node();
        const firstChild = node(first);
        const second = node();
        expect(revealTarget([second, firstChild, first])).toBe(first);
        expect(revealTarget([second, firstChild])).toBe(firstChild);
    });

    it("is null with nothing to reveal", () => {
        expect(revealTarget([])).toBeNull();
    });
});
