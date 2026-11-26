// mailto links in the note and invoice frames, handed to the main window.
//
// The frames are sandboxed with `allow-same-origin` only, and WebKit refuses
// any custom-protocol load (mailto: included) that comes from a sandboxed
// frame, before the desktop window's navigation handler can see it. A plain
// link navigates the frame itself, so `allow-top-navigation-to-custom-protocols`
// does not help either. The frames are same-origin, though, so this window
// can listen to clicks in the frame's document: a click on a mailto link is
// cancelled there and becomes a navigation of THIS window, which the desktop
// window hands to the mail app (and cancels), and a plain browser
// (`--server` mode) gives to its mail client while the page stays.
//
// Every other link is left alone: https already leaves through the shell's
// `frame-src`, our own pages load in the frame, and the rest stay blocked.

/** How the main window follows a link; replaced in tests. */
export type Navigate = (href: string) => void;

const followInMainWindow: Navigate = (href) => {
    window.location.href = href;
};

/** The `<a href>` a click landed in, if any. Not `instanceof`: the frame's
 *  nodes belong to its own realm. */
function anchorOf(target: EventTarget | null): HTMLAnchorElement | null {
    const element = target as Element | null;
    if (typeof element?.closest !== "function") return null;
    return element.closest("a[href]") as HTMLAnchorElement | null;
}

function isMailto(href: string): boolean {
    try {
        return new URL(href).protocol === "mailto:";
    } catch {
        return false;
    }
}

/**
 * Wire the document `frame` holds right now; call it from the frame's `load`
 * event. Each load is a new document, so nothing is ever wired twice, and a
 * frame with no readable document (none yet, or not same-origin) is skipped.
 */
export function handMailtoLinks(frame: HTMLIFrameElement, navigate: Navigate = followInMainWindow): void {
    const doc = frame.contentDocument;
    if (!doc) return;
    doc.addEventListener("click", (event) => {
        if (event.defaultPrevented) return;
        const anchor = anchorOf(event.target);
        if (!anchor || !isMailto(anchor.href)) return;
        event.preventDefault();
        navigate(anchor.href);
    });
}
