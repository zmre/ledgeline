// Native mode: the SPA running inside the iOS/iPadOS app (plan 42).
//
// The app's document-start user script sets BOTH markers before any page
// script runs:
//
//     window.__LEDGELINE_NATIVE__ = {platform: "ios", idiom: "phone" | "pad", version: 1};
//     document.documentElement.classList.add("ll-native");
//
// Either one is enough to count as native, so a page reloaded by the web view
// (where the class survives a re-run of the script in any order) still knows.
// Nothing server-side changes: the same bundle serves the desktop app, the
// browser and the phone. Every native-only path in the app is gated on
// `isNative()` and is a no-op everywhere else.
//
// The names here are referenced from Swift STRING LITERALS (see
// `contract.test.ts`), so a rename compiles on both sides and silently breaks
// the app. Change them only together with apple/ios.

/** The global the app's user script sets. */
export const NATIVE_GLOBAL = "__LEDGELINE_NATIVE__";
/** The class the app's user script puts on `<html>`; the CSS keys off it. */
export const NATIVE_CLASS = "ll-native";
/** The `WKScriptMessageHandler` name the app registers. */
export const MESSAGE_HANDLER = "ledgeline";
/** The object the app calls into (`window.ledgelineNative.back()` etc.). */
export const NATIVE_API = "ledgelineNative";

export interface NativeInfo {
    platform: "ios";
    idiom: "phone" | "pad";
    version: number;
}

interface MessageHandler {
    postMessage(message: unknown): void;
}

declare global {
    interface Window {
        __LEDGELINE_NATIVE__?: NativeInfo;
        webkit?: {messageHandlers?: Record<string, MessageHandler | undefined>};
    }
}

/** Whether the page is shown inside the iOS/iPadOS app. False wherever there is no window. */
export function isNative(): boolean {
    if (typeof window === "undefined") return false;
    if (window[NATIVE_GLOBAL] !== undefined && window[NATIVE_GLOBAL] !== null) return true;
    return typeof document !== "undefined" && document.documentElement.classList.contains(NATIVE_CLASS);
}

/** What the app said about itself, or null outside it (or when only the class is set). */
export function nativeInfo(): NativeInfo | null {
    if (typeof window === "undefined") return null;
    return window[NATIVE_GLOBAL] ?? null;
}

/**
 * Sends one message to the app. False outside it, or when the handler is not
 * installed (an app build that predates the message, a test).
 */
export function postNative(message: {type: string} & Record<string, unknown>): boolean {
    if (!isNative()) return false;
    const handler = window.webkit?.messageHandlers?.[MESSAGE_HANDLER];
    if (handler === undefined) return false;
    handler.postMessage(message);
    return true;
}
