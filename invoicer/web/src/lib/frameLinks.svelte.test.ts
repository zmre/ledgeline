// The note and invoice frames' mailto links go to the main window; every
// other link is left to the frame. jsdom gives a same-origin iframe a real
// document, as the app's `allow-same-origin` frames have.
import {afterEach, describe, expect, it, vi} from "vitest";
import {handMailtoLinks} from "./frameLinks";

function frameWith(html: string): {frame: HTMLIFrameElement; doc: Document} {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const doc = frame.contentDocument!;
    doc.body.innerHTML = html;
    return {frame, doc};
}

/** Click `selector` in `doc`; answers whether the click was cancelled. */
function click(doc: Document, selector: string): boolean {
    const element = doc.querySelector(selector) as HTMLElement;
    const event = new doc.defaultView!.MouseEvent("click", {bubbles: true, cancelable: true});
    // jsdom does not navigate; stop it from trying (and logging) for links
    // the helper leaves alone. Runs after the document's listener.
    doc.defaultView!.addEventListener("click", (e) => e.preventDefault(), {once: true});
    element.dispatchEvent(event);
    return event.defaultPrevented;
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("mailto links in a frame", () => {
    it("cancels a mailto click and navigates the main window to it", () => {
        const {frame, doc} = frameWith(`<a id="m" href="mailto:ap@example.com?subject=Invoice%20108">Email</a>`);
        const navigate = vi.fn();
        handMailtoLinks(frame, navigate);
        let prevented = false;
        doc.addEventListener("click", (e) => (prevented = e.defaultPrevented));
        click(doc, "#m");
        expect(prevented).toBe(true);
        expect(navigate).toHaveBeenCalledExactlyOnceWith("mailto:ap@example.com?subject=Invoice%20108");
    });

    it("follows a click on an element inside the link", () => {
        const {frame, doc} = frameWith(`<a href="mailto:ap@example.com"><strong id="s">ap</strong></a>`);
        const navigate = vi.fn();
        handMailtoLinks(frame, navigate);
        click(doc, "#s");
        expect(navigate).toHaveBeenCalledExactlyOnceWith("mailto:ap@example.com");
    });

    it("leaves https, relative, tel and plain clicks to the frame", () => {
        const {frame, doc} = frameWith(
            `<a id="h" href="https://example.com/">web</a>
             <a id="r" href="/invoices/108/">page</a>
             <a id="t" href="tel:+15551234567">call</a>
             <a id="n">no href</a>
             <p id="p">text</p>`
        );
        const navigate = vi.fn();
        handMailtoLinks(frame, navigate);
        const seen: boolean[] = [];
        doc.addEventListener("click", (e) => seen.push(e.defaultPrevented));
        for (const id of ["#h", "#r", "#t", "#n", "#p"]) click(doc, id);
        expect(seen).toEqual([false, false, false, false, false]);
        expect(navigate).not.toHaveBeenCalled();
    });

    it("ignores a click something already cancelled", () => {
        const {frame, doc} = frameWith(`<a id="m" href="mailto:ap@example.com"><span id="s">ap</span></a>`);
        doc.querySelector("#s")!.addEventListener("click", (e) => e.preventDefault());
        const navigate = vi.fn();
        handMailtoLinks(frame, navigate);
        click(doc, "#s");
        expect(navigate).not.toHaveBeenCalled();
    });

    it("skips a frame with no document", () => {
        const frame = document.createElement("iframe");
        expect(frame.contentDocument).toBeNull();
        expect(() => handMailtoLinks(frame, vi.fn())).not.toThrow();
    });
});
