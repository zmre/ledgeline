// Page → app messages that are not page state: the share sheet and haptics
// (plan 42). Both are no-ops outside the app and say so by returning false, so
// a caller can fall back to the browser's own path.

import {postNative} from "./detect";

/** Base64 of `bytes`, chunked so a large workbook does not overflow `String.fromCharCode`'s argument list. */
export function toBase64(bytes: Uint8Array): string {
    const CHUNK = 0x8000;
    let binary = "";
    for (let at = 0; at < bytes.length; at += CHUNK) {
        binary += String.fromCharCode(...bytes.subarray(at, at + CHUNK));
    }
    return btoa(binary);
}

/**
 * Hands a file to the app's share sheet (Save to Files, AirDrop, Mail, …)
 * instead of an `<a download>`, which a web view cannot complete. False
 * outside the app: the caller downloads it the browser's way.
 */
export function share(filename: string, mime: string, bytes: Uint8Array | ArrayBuffer): boolean {
    const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    return postNative({type: "share", filename, mime, base64: toBase64(view)});
}

export type HapticStyle = "light" | "success" | "warning";

/** Optional feedback; false (and nothing happens) outside the app. */
export function haptic(style: HapticStyle): boolean {
    return postNative({type: "haptic", style});
}
