// Mounting the review overlay (plan 32 Phase 3) against the fake review
// engine: a card's wire, a suggestion's wire, and keys pressed the way the app
// delivers them — dispatched at the focused element (so element handlers and
// `dismissible`'s document listener run) and then handed to the keymap, which
// the layout's `<svelte:window onkeydown>` would do in the app.

import {render, screen} from "@testing-library/svelte";
import {tick} from "svelte";
import {vi} from "vitest";
import {keymap} from "#lib/keys/keymap.svelte.js";
import type {QueueKind} from "#lib/review/types.js";
import ReviewOverlay from "#lib/review/ui/ReviewOverlay.svelte";
import {review} from "#lib/stores/review.svelte.js";

/** A suggestion as the queue serves it. */
export function suggestionWire(account: string, strong = true, extra: Record<string, unknown> = {}): Record<string, unknown> {
    return {account, description: null, source: "exactKey", confidence: strong ? 8333 : 3000, strong, count: 5, ...extra};
}

/** Mount the overlay over `kind` and wait for its queue (and first card, when there is one). */
export async function mountOverlay(kind: QueueKind = "unknowns"): Promise<void> {
    render(ReviewOverlay, {props: {kind}});
    await vi.waitFor(() => {
        if (review.loading || review.kind === null) throw new Error("queue not loaded");
    });
    await settle();
}

/** Let effects, the focus tick and any microtask-resolved fetch land. */
export async function settle(): Promise<void> {
    for (let i = 0; i < 4; i += 1) {
        await tick();
        await Promise.resolve();
    }
}

/** Press `key` on whatever has focus, as the browser and then the window keymap would see it. */
export async function press(key: string, init: KeyboardEventInit = {}): Promise<KeyboardEvent> {
    const target = document.activeElement ?? document.body;
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true, ...init});
    target.dispatchEvent(event);
    keymap.handle(event);
    await settle();
    return event;
}

/** ⌘/Ctrl, whichever `mod+` means here (chord.ts reads the platform the same way). */
export const MOD: KeyboardEventInit = /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent) ? {metaKey: true} : {ctrlKey: true};

export const card = (): HTMLElement => screen.getByTestId("review-card");
export const footer = (): HTMLElement => screen.getByTestId("review-footer");
export const primaryButton = (): HTMLButtonElement => screen.getByTestId("review-primary") as HTMLButtonElement;
export const accountField = (row = 0): HTMLInputElement => screen.getByLabelText(`Account for posting ${row + 1}`) as HTMLInputElement;
