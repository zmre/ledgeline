// The invoice frame, mounted: MBR's page or a preview page, always by URL
// (never srcdoc, which inherits the app's CSP), and never a script-running
// page (a template must not reach the app's window).
import {render, screen} from "@testing-library/svelte";
import {describe, expect, it} from "vitest";
import InvoiceFrame from "./InvoiceFrame.svelte";

describe("COMPONENT InvoiceFrame", () => {
    it("frames MBR's page for a saved invoice, sandboxed without scripts", () => {
        render(InvoiceFrame, {src: "/invoices/A%20B/", title: "Invoice 105"});
        const frame = screen.getByTestId("invoice-frame");
        expect(frame.getAttribute("src")).toBe("/invoices/A%20B/");
        expect(frame.getAttribute("sandbox")).toBe("allow-same-origin");
        expect(frame.getAttribute("title")).toBe("Invoice 105");
        expect(frame.dataset.mode).toBe("page");
    });

    it("frames a preview page by URL too, never srcdoc", () => {
        render(InvoiceFrame, {src: "/.ledgeline-invoicer/preview?number=105&template=%2Ft.html", title: "Preview", mode: "preview"});
        const frame = screen.getByTestId("invoice-frame");
        expect(frame.getAttribute("src")).toBe("/.ledgeline-invoicer/preview?number=105&template=%2Ft.html");
        expect(frame.hasAttribute("srcdoc")).toBe(false);
        expect(frame.getAttribute("sandbox")).toBe("allow-same-origin");
        expect(frame.dataset.mode).toBe("preview");
    });

    it("renders nothing with nothing to show", () => {
        render(InvoiceFrame, {title: "Nothing"});
        expect(screen.queryByTestId("invoice-frame")).toBeNull();
    });
});
