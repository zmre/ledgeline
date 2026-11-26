// mailto links in the note and invoice frames, handed to the desktop window
// through our own origin.
//
// The frames are sandboxed with `allow-same-origin` only. WebKit refuses
// every custom-protocol load (mailto: included) from such a frame, runs no
// script in it (not even this window's listeners), and does not honour
// `allow-top-navigation-to-custom-protocols`. What it does allow is a
// same-origin navigation, and this window may still edit the frame's DOM.
// So on each load, every mailto link in the frame is rewritten to the
// hand-off path, `HANDOFF_PATH?url=<the link>`: the desktop window's
// navigation handler recognises it, opens the link in the mail app and
// keeps the frame where it is (crates/ledgeline-shell/src/gui.rs
// `handoff_target`). In `--server` mode, in a plain browser, the server
// answers the hand-off with 204 No Content: the frame stays put and nothing
// opens.
//
// Every other link is left alone: https leaves through the shell's
// `frame-src`, our own pages load in the frame, and the rest stay blocked.

/**
 * The hand-off path. MUST match `HANDOFF_PATH` in
 * crates/ledgeline-shell/src/security.rs (both sides pin the string in a test).
 */
export const HANDOFF_PATH = "/.ledgeline/open";

/** The same-origin hand-off URL for `href` (relative to the frame's origin). */
export function handoffHref(href: string): string {
    return `${HANDOFF_PATH}?url=${encodeURIComponent(href)}`;
}

function isMailto(href: string): boolean {
    try {
        return new URL(href).protocol === "mailto:";
    } catch {
        return false;
    }
}

/**
 * Rewrite the mailto links of the document `frame` holds right now; call it
 * from the frame's `load` event (each load is a new document). A frame with
 * no readable document (none yet, or not same-origin) is skipped. Calling it
 * twice changes nothing more: a rewritten link is same-origin, not mailto.
 */
export function handMailtoLinks(frame: HTMLIFrameElement): void {
    const doc = frame.contentDocument;
    if (!doc) return;
    for (const anchor of Array.from(doc.querySelectorAll<HTMLAnchorElement>("a[href]"))) {
        if (isMailto(anchor.href)) anchor.setAttribute("href", handoffHref(anchor.href));
    }
}
