// The note and invoice frames' mailto links become same-origin hand-offs;
// every other link is left as it is. jsdom gives a same-origin iframe a real
// document, as the app's `allow-same-origin` frames have.
import {afterEach, describe, expect, it} from "vitest";
import {HANDOFF_PATH, handMailtoLinks, handoffHref} from "./frameLinks";

function frameWith(html: string): {frame: HTMLIFrameElement; doc: Document} {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const doc = frame.contentDocument!;
    doc.body.innerHTML = html;
    return {frame, doc};
}

const hrefOf = (doc: Document, selector: string) => doc.querySelector(selector)!.getAttribute("href");

afterEach(() => {
    document.body.innerHTML = "";
});

describe("mailto links in a frame", () => {
    it("uses the shell's hand-off path", () => {
        // crates/ledgeline-shell/src/gui.rs `the_handoff_path_is_pinned` pins the same string.
        expect(HANDOFF_PATH).toBe("/.ledgeline/open");
    });

    it("rewrites a mailto link to the hand-off, fully encoded", () => {
        const {frame, doc} = frameWith(`<a id="m" href="mailto:ap@example.com?subject=Invoice%20108&amp;body=Hi">Email</a>`);
        handMailtoLinks(frame);
        expect(hrefOf(doc, "#m")).toBe("/.ledgeline/open?url=mailto%3Aap%40example.com%3Fsubject%3DInvoice%2520108%26body%3DHi");
        const target = new URL(hrefOf(doc, "#m")!, "http://127.0.0.1:5000").searchParams.get("url");
        expect(target).toBe("mailto:ap@example.com?subject=Invoice%20108&body=Hi");
    });

    it("rewrites a link with markup inside, keeping the markup", () => {
        const {frame, doc} = frameWith(`<a id="m" href="MAILTO:ap@example.com"><strong>ap</strong></a>`);
        handMailtoLinks(frame);
        expect(hrefOf(doc, "#m")).toBe(handoffHref("mailto:ap@example.com"));
        expect(doc.querySelector("#m strong")?.textContent).toBe("ap");
    });

    it("leaves https, relative, tel and href-less links alone", () => {
        const html = `<a id="h" href="https://example.com/">web</a>
             <a id="r" href="/invoices/108/">page</a>
             <a id="t" href="tel:+15551234567">call</a>
             <a id="n">no href</a>`;
        const {frame, doc} = frameWith(html);
        handMailtoLinks(frame);
        expect(hrefOf(doc, "#h")).toBe("https://example.com/");
        expect(hrefOf(doc, "#r")).toBe("/invoices/108/");
        expect(hrefOf(doc, "#t")).toBe("tel:+15551234567");
        expect(hrefOf(doc, "#n")).toBeNull();
    });

    it("never wraps a link twice", () => {
        const {frame, doc} = frameWith(`<a id="m" href="mailto:ap@example.com">Email</a>`);
        handMailtoLinks(frame);
        const once = hrefOf(doc, "#m");
        handMailtoLinks(frame);
        expect(hrefOf(doc, "#m")).toBe(once);
    });

    it("skips a frame with no document", () => {
        const frame = document.createElement("iframe");
        expect(frame.contentDocument).toBeNull();
        expect(() => handMailtoLinks(frame)).not.toThrow();
    });
});
