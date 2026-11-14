// The page's half of the iOS/iPadOS app bridge (plan 42). Inert everywhere but
// the app.
//
// In the app the HTML header is collapsed (`html.ll-native` in +layout.svelte)
// and a native navigation bar takes its place: the section title, a back
// button, and a hamburger menu holding the sections and the page's actions.
// This module feeds that bar and answers it.
//
// - **Page → app:** `{type: "page", title, sections, actions, canGoBack}`,
//   posted by `publishPage` from an effect in the root layout, so it goes out
//   after every navigation and whenever an action, an overlay or the history
//   depth changes. Deduped: the same payload twice is posted once.
// - **App → page:** `window.ledgelineNative.{navigate, action, back, state}`.
//
// Pages contribute actions with `registerNativeActions(() => [...])` during
// component init; the getter is re-read reactively, so a label carrying a count
// or an action that comes and goes re-posts the menu. Overlays register a close
// handler with `registerNativeOverlay` (or get one for free through the
// `dismissible` action) so that `back()` closes the topmost one before it
// touches history.
//
// Nothing here imports `$app/*`: the router is handed in by the layout
// (`installNativeBridge({navigate})`), which is what lets the node test project
// exercise all of it.

import {untrack} from "svelte";
import {isNative, NATIVE_API, postNative} from "./detect";

/** Every section the app's menu can show, in menu order. Imports is never one (plan 42). */
export const SECTION_IDS = ["journal", "holdings", "budget", "projections", "reports", "settings"] as const;
export type SectionId = (typeof SECTION_IDS)[number];

export function isSectionId(id: string): id is SectionId {
    return (SECTION_IDS as readonly string[]).includes(id);
}

export interface NativeSection {
    id: SectionId;
    label: string;
    active: boolean;
}

/** An entry in the app's menu. `icon` is an SF Symbol name; the app shows none when it does not know it. */
export interface NativeActionInfo {
    id: string;
    label: string;
    icon?: string;
    destructive?: boolean;
}

export interface NativeAction extends NativeActionInfo {
    run: () => void;
}

export interface PageMessage {
    type: "page";
    title: string;
    sections: NativeSection[];
    actions: NativeActionInfo[];
    canGoBack: boolean;
}

/** What the layout knows and the bridge does not: where we are. */
export interface PageMeta {
    title: string;
    sections: NativeSection[];
}

export interface NativeApi {
    navigate(sectionId: string): boolean;
    action(actionId: string): boolean;
    back(): boolean;
    state(): PageMessage | null;
}

declare global {
    interface Window {
        ledgelineNative?: NativeApi;
    }
}

// ---------------------------------------------------------------------------
// Actions

/** Global actions (refresh, problems) sit after the page's own. */
export const GLOBAL_ACTION_ORDER = 100;

interface ActionProvider {
    seq: number;
    order: number;
    get: () => readonly NativeAction[];
}

// `$state.raw`: entries are replaced wholesale and compared by `seq`, never
// proxied (see keymap.svelte.ts on why proxies and identity filters do not mix).
let providers = $state.raw<ActionProvider[]>([]);
let seq = 0;

/**
 * Adds a provider of menu actions and returns its removal. Imperative and
 * untracked, so it is safe to call from an effect; `registerNativeActions` is
 * the component-scoped wrapper.
 */
export function addActionProvider(get: () => readonly NativeAction[], order = 0): () => void {
    seq += 1;
    const id = seq;
    untrack(() => {
        providers = [...providers, {seq: id, order, get}];
    });
    return () => {
        untrack(() => {
            providers = providers.filter((p) => p.seq !== id);
        });
    };
}

/**
 * Contributes actions to the app's menu for as long as the calling component
 * is mounted. Call during component init. The getter is reactive: whatever
 * state it reads re-posts the menu when it changes. No-op outside the app.
 */
export function registerNativeActions(get: () => readonly NativeAction[], options: {order?: number} = {}): void {
    if (!isNative()) return;
    $effect(() => addActionProvider(get, options.order ?? 0));
}

/** The menu's actions right now: by order, then registration; the first of a repeated id wins. */
export function currentActions(): NativeAction[] {
    const sorted = [...providers].sort((a, b) => a.order - b.order || a.seq - b.seq);
    const out: NativeAction[] = [];
    for (const provider of sorted) {
        for (const action of provider.get()) {
            if (!out.some((kept) => kept.id === action.id)) out.push(action);
        }
    }
    return out;
}

/** Runs the action `id`; false when the page offers no such action now. */
export function runAction(id: string): boolean {
    const action = untrack(() => currentActions()).find((a) => a.id === id);
    if (action === undefined) return false;
    action.run();
    return true;
}

// ---------------------------------------------------------------------------
// Overlays

interface Overlay {
    seq: number;
    close: () => void;
}

let overlays = $state.raw<Overlay[]>([]);

/**
 * Marks an overlay open until the returned function is called. `back()`
 * calls the most recently opened one's `close` before it goes back in
 * history. Untracked, so `dismissible`'s attach/detach can call it directly.
 */
export function addOverlay(close: () => void): () => void {
    seq += 1;
    const id = seq;
    untrack(() => {
        overlays = [...overlays, {seq: id, close}];
    });
    return () => {
        untrack(() => {
            overlays = overlays.filter((o) => o.seq !== id);
        });
    };
}

/**
 * An overlay that is open while `isOpen()` says so (reactive), closed by
 * `close`. Call during component init. No-op outside the app. Overlays built
 * on `use:dismissible` need not call this: that action registers itself.
 */
export function registerNativeOverlay(isOpen: () => boolean, close: () => void): void {
    if (!isNative()) return;
    $effect(() => {
        if (!isOpen()) return;
        return addOverlay(close);
    });
}

/** How many overlays are open. */
export function openOverlays(): number {
    return overlays.length;
}

// ---------------------------------------------------------------------------
// History

/**
 * In-app history depth: how many entries back the first page of this
 * document is.
 *
 * SvelteKit numbers its history entries (`historyIndex`, +1 on a push, +0 on
 * a replace, and restored on popstate), so when that number is readable the
 * depth is simply the distance from the first page's — exact across
 * `replace: true` redirects and the back button alike. It lives in
 * `history.state["sveltekit:metadata"]`, which is not public API; when it is
 * not there, navigations are counted instead (a push +1, a popstate by its
 * delta), which only overcounts a `goto(…, {replace: true})`.
 */
export class HistoryDepth {
    #base: number | null = null;
    #count = 0;

    /** Records one completed navigation; returns the new depth. */
    note(type: string, delta: number | undefined, index: number | null): number {
        if (type === "enter") {
            this.#base = index;
            this.#count = 0;
            return 0;
        }
        if (this.#base !== null && index !== null) {
            this.#count = Math.max(0, index - this.#base);
            return this.#count;
        }
        if (type === "popstate") this.#count = Math.max(0, this.#count + (delta ?? -1));
        else this.#count += 1;
        return this.#count;
    }
}

/** SvelteKit's index of the current history entry, or null when it is not where kit 3 keeps it. */
export function sveltekitHistoryIndex(state: unknown = typeof history === "undefined" ? null : history.state): number | null {
    if (state === null || typeof state !== "object") return null;
    const meta = (state as Record<string, unknown>)["sveltekit:metadata"];
    if (meta === null || typeof meta !== "object") return null;
    const index = (meta as Record<string, unknown>).historyIndex;
    return typeof index === "number" && Number.isFinite(index) ? index : null;
}

const depthTracker = new HistoryDepth();
let depth = $state(0);

/** Called from the layout's `afterNavigate`. */
export function noteNavigation(type: string, delta?: number, index: number | null = sveltekitHistoryIndex()): void {
    depth = depthTracker.note(type, delta, index);
}

export function historyDepth(): number {
    return depth;
}

/**
 * The app's back button: closes the topmost overlay, else goes back one entry
 * in in-app history. False when there is nothing to go back to, which tells
 * the app to show its home screen.
 */
export function back(): boolean {
    const top = untrack(() => overlays[overlays.length - 1]);
    if (top !== undefined) {
        top.close();
        return true;
    }
    if (untrack(() => depth) > 0) {
        history.back();
        return true;
    }
    return false;
}

// ---------------------------------------------------------------------------
// Page state

let lastMessage: PageMessage | null = null;
let lastKey = "";

/** The `page` message for `meta` and the current actions, overlays and history. Reactive in all of them. */
export function pageMessage(meta: PageMeta): PageMessage {
    return {
        type: "page",
        title: meta.title,
        sections: meta.sections.map(({id, label, active}) => ({id, label, active})),
        actions: currentActions().map(({id, label, icon, destructive}) => ({
            id,
            label,
            ...(icon === undefined ? {} : {icon}),
            ...(destructive === true ? {destructive: true} : {}),
        })),
        canGoBack: overlays.length > 0 || depth > 0,
    };
}

/**
 * Posts the page's state to the app unless it is the same as last time.
 * Returns whether a message went out. Call from an effect: everything
 * `pageMessage` reads becomes a dependency, so a change re-posts.
 */
export function publishPage(meta: PageMeta): boolean {
    if (!isNative()) return false;
    const message = pageMessage(meta);
    const key = JSON.stringify(message);
    if (key === lastKey) return false;
    lastKey = key;
    lastMessage = message;
    return postNative({...message});
}

/** The last `page` message posted, for the app to ask again. */
export function lastPageMessage(): PageMessage | null {
    return lastMessage;
}

/**
 * Installs `window.ledgelineNative` (in the app only) and returns its
 * removal. `navigate` is the layout's router hook: it receives a known
 * section id and moves there.
 */
export function installNativeBridge(deps: {navigate: (id: SectionId) => void}): () => void {
    if (!isNative()) return () => {};
    const api: NativeApi = {
        navigate(sectionId: string): boolean {
            if (!isSectionId(sectionId)) return false;
            deps.navigate(sectionId);
            return true;
        },
        action: (actionId: string) => runAction(actionId),
        back: () => back(),
        state: () => lastPageMessage(),
    };
    window[NATIVE_API] = api;
    return () => {
        if (window[NATIVE_API] === api) delete window[NATIVE_API];
    };
}

/** Forgets every registration and the last post. Tests only. */
export function resetNativeBridge(): void {
    providers = [];
    overlays = [];
    depth = 0;
    lastMessage = null;
    lastKey = "";
    depthTracker.note("enter", undefined, null);
}
