// Unsaved edits on a page (Phase 4 feedback H13, review TW-7/TW-8): leaving
// asks Save / Discard / Cancel in SaveDiscardDialog, and so does anything
// else that would drop them (another catalog, Cancel editing). One guard for
// every editing page, so each behaves the same:
//
// - A link, Back or `goto` to another page is held while the dialog asks.
//   The same page with another query or hash (the New menu's `?new=1`) isn't
//   leaving: the page stays mounted, nothing is lost.
// - Closing the window or reloading is the browser's own prompt
//   (`beforeunload`; SvelteKit's "leave" navigation is left alone).
// - Save, then go on; a save that fails keeps the dialog open with why. A
//   save that asks something else first (a sent invoice's re-book) closes it:
//   that dialog takes over, and the user goes on from there.
// - Discard, then go on. The step the guard itself carries out (its own
//   `goto`) is never asked about again.
import {onMount} from "svelte";
import {beforeNavigate, goto} from "$app/navigation";
import {page} from "$app/state";

/** A save's outcome: done, refused (say why), or waiting on another question. */
export type SaveOutcome = "saved" | "failed" | "asked";

export type GuardOptions = Readonly<{
    /** Whether there is anything unsaved to lose. */
    dirty: () => boolean;
    save: () => Promise<SaveOutcome>;
    /** Drop the unsaved edits. */
    discard: () => void;
    /** Why the last save failed, in the user's words (shown in the dialog). */
    saveError: () => string | null;
    /** The file changed on disk under these edits (a refused save). */
    conflict?: () => boolean;
    /** What is being edited, for the conflict sentence: "template". */
    noun: string;
}>;

/** What waits on the user's choice; `leaving` when it is another page. */
type Pending = Readonly<{then: () => unknown; leaving: boolean}>;

/** The part of SvelteKit's `BeforeNavigate` the guard reads. */
export type NavigationLike = Readonly<{type: string; to: {url: URL} | null; cancel: () => void}>;

/** Where the page is now (`page.url`). */
type Here = Pick<URL, "origin" | "pathname">;

export class UnsavedGuard {
    /** The step waiting on Save / Discard / Cancel: the dialog is open while set. */
    pending = $state<Pending | null>(null);
    /** Why Save didn't go on (shown in the dialog). */
    error = $state<string | null>(null);

    readonly #options: GuardOptions;
    readonly #navigate: (href: string) => unknown;
    readonly #here: () => Here;
    /** The guard's own step is under way: its navigation isn't asked about. */
    #passing = false;

    constructor(options: GuardOptions, navigate: (href: string) => unknown, here: () => Here) {
        this.#options = options;
        this.#navigate = navigate;
        this.#here = here;
    }

    /** Run `then` now, or once the user has saved or discarded the unsaved edits. */
    proceed(then: () => unknown): void {
        if (!this.#options.dirty()) {
            void then();
            return;
        }
        this.#ask({then, leaving: false});
    }

    /** Cancel: stay, edits and all. */
    cancel(): void {
        this.pending = null;
        this.error = null;
    }

    /** Save, then go on (or say why not). */
    async save(): Promise<void> {
        const asked = this.pending;
        if (asked === null) return;
        this.error = null;
        const outcome = await this.#options.save();
        if (this.pending !== asked) return; // cancelled meanwhile
        if (outcome === "saved") await this.#goOn(asked);
        else if (outcome === "asked") this.cancel();
        else this.error = this.#whyNot();
    }

    /** Discard the edits, then go on. */
    async discard(): Promise<void> {
        const asked = this.pending;
        if (asked === null) return;
        this.#options.discard();
        await this.#goOn(asked);
    }

    /** SvelteKit's `beforeNavigate`: hold a navigation off the page over unsaved edits. */
    beforeNavigate(navigation: NavigationLike): void {
        const to = navigation.to;
        if (this.#passing || to === null || navigation.type === "leave" || !this.#options.dirty()) return;
        const here = this.#here();
        if (to.url.origin === here.origin && to.url.pathname === here.pathname) return;
        navigation.cancel();
        const href = to.url.origin === here.origin ? to.url.pathname + to.url.search + to.url.hash : to.url.href;
        this.#ask({then: () => this.#navigate(href), leaving: true});
    }

    /** `beforeunload`: the browser's own prompt over unsaved edits. */
    beforeUnload(event: Pick<BeforeUnloadEvent, "preventDefault">): void {
        if (this.#options.dirty()) event.preventDefault();
    }

    #ask(pending: Pending): void {
        this.pending = pending;
        this.error = null;
    }

    async #goOn(asked: Pending): Promise<void> {
        this.cancel();
        this.#passing = true;
        try {
            await asked.then();
        } finally {
            this.#passing = false;
        }
    }

    #whyNot(): string {
        const error = this.#options.saveError();
        if (error !== null) return error;
        if (this.#options.conflict?.() === true) return `The ${this.#options.noun} changed on disk; reload it or keep yours first.`;
        return "It couldn't be saved.";
    }
}

/** Go to `href`: an app URL through the router, anything else by the browser. */
function navigate(href: string): unknown {
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- a URL SvelteKit itself was navigating to
    if (href.startsWith("/")) return goto(href);
    window.location.assign(href);
    return undefined;
}

/**
 * The page's guard, wired to SvelteKit's navigation and the window's
 * `beforeunload`. Call it while the page initialises; draw SaveDiscardDialog
 * while `pending` is set, with `save`, `discard`, `cancel` and `error`.
 */
export function unsavedGuard(options: GuardOptions): UnsavedGuard {
    const guard = new UnsavedGuard(options, navigate, () => page.url);
    beforeNavigate((navigation) => guard.beforeNavigate(navigation));
    onMount(() => {
        const warn = (event: BeforeUnloadEvent): void => guard.beforeUnload(event);
        window.addEventListener("beforeunload", warn);
        return () => window.removeEventListener("beforeunload", warn);
    });
    return guard;
}
