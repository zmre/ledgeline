// Apply payment's dialog (cash books, I17), mounted over a fake server: the
// deposits listed best first, the first chosen and previewed (a dry run),
// another choice previewed in turn, Apply payment off until a preview is in,
// and the apply itself sent without dry_run.
import {fireEvent, render, screen, waitFor, within} from "@testing-library/svelte";
import {afterEach, describe, expect, it, vi} from "vitest";
import ApplyPaymentDialog from "./ApplyPaymentDialog.svelte";

const candidate = (reference: string, description: string, received: string, exact: boolean) => ({
    reference,
    date: "2026-11-17",
    description,
    received,
    exact,
    by_number: description.includes("207"),
    by_customer: true,
    gap_days: 1,
});

const OPTIONS = {
    number: "207",
    commodity: "$",
    total: "$7,000.00",
    paid: "$0.00",
    remaining: "$7,000.00",
    candidates: [
        candidate("aaaaaaaaaaaa", "WIRE CUSTOMER X INV 207", "$3,500.00", false),
        candidate("bbbbbbbbbbbb", "WIRE CUSTOMER X BALANCE", "$7,000.00", true),
    ],
};

const result = (reference: string, written: boolean) =>
    reference === "aaaaaaaaaaaa"
        ? {
              written,
              number: "207",
              date: "2026-11-17",
              description: "WIRE CUSTOMER X INV 207",
              received: "$3,500.00",
              applied: [
                  {account: "revenue:consulting", amount: "$1,500.00"},
                  {account: "revenue:consulting:delivery", amount: "$2,000.00"},
              ],
              partial: true,
              remaining: "$3,500.00",
              warnings: [],
          }
        : {
              written,
              number: "207",
              date: "2026-11-17",
              description: "WIRE CUSTOMER X BALANCE",
              received: "$7,000.00",
              applied: [
                  {account: "revenue:consulting", amount: "$3,000.00"},
                  {account: "revenue:consulting:delivery", amount: "$4,000.00"},
              ],
              partial: false,
              remaining: "$0.00",
              warnings: [],
          };

/** The server: GET lists, POST answers the dry run or the apply. */
function server(options: unknown = OPTIONS) {
    const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
        if ((init?.method ?? "GET") === "GET") return new Response(JSON.stringify(options));
        const body = JSON.parse(String(init?.body)) as {reference: string; dry_run: boolean};
        return new Response(JSON.stringify(result(body.reference, !body.dry_run)));
    });
    vi.stubGlobal("fetch", fetch);
    return fetch;
}

const posts = (fetch: ReturnType<typeof server>) =>
    fetch.mock.calls.filter(([, init]) => init?.method === "POST").map(([, init]) => JSON.parse(String(init?.body)) as Record<string, unknown>);

const shares = () =>
    screen.getAllByTestId("apply-payment-share").map((row) => Array.from(row.querySelectorAll("td"), (td) => (td.textContent ?? "").trim()).join(" "));

describe("COMPONENT ApplyPaymentDialog", () => {
    afterEach(() => {
        vi.unstubAllGlobals();
    });

    it("previews the best deposit, focused, then applies the one chosen", async () => {
        const fetch = server();
        const onApplied = vi.fn();
        render(ApplyPaymentDialog, {number: "207", onApplied, onCancel: vi.fn()});
        const confirm = screen.getByTestId("apply-payment-confirm") as HTMLButtonElement;
        expect(confirm.disabled).toBe(true);

        await waitFor(() => expect(screen.getByTestId("apply-payment-outcome").textContent).toBe("Pays $3,500.00; $3,500.00 still due."));
        expect(screen.getByTestId("apply-payment-due").textContent?.trim()).toBe("$7,000.00 still due.");
        const radios = screen.getAllByRole("radio") as HTMLInputElement[];
        expect(radios.map((r) => r.checked)).toEqual([true, false]);
        expect(document.activeElement).toBe(radios[0]);
        expect(within(screen.getAllByTestId("apply-payment-candidate")[0]).getByText(/Names this invoice/)).toBeDefined();
        expect(shares()).toEqual(["revenue:consulting $1,500.00", "revenue:consulting:delivery $2,000.00"]);
        // No reference is ever shown.
        expect(document.body.textContent).not.toContain("aaaaaaaaaaaa");
        expect(confirm.disabled).toBe(false);

        // Another choice: previewed in turn.
        await fireEvent.click(radios[1]);
        await waitFor(() => expect(screen.getByTestId("apply-payment-outcome").textContent).toBe("Pays $7,000.00: paid in full."));
        expect(shares()).toEqual(["revenue:consulting $3,000.00", "revenue:consulting:delivery $4,000.00"]);

        await fireEvent.click(confirm);
        await waitFor(() => expect(onApplied).toHaveBeenCalledOnce());
        expect(onApplied.mock.calls[0][0]).toMatchObject({written: true, remaining: "$0.00"});
        expect(posts(fetch).map((b) => [b.reference, b.dry_run])).toEqual([
            ["aaaaaaaaaaaa", true],
            ["bbbbbbbbbbbb", true],
            ["bbbbbbbbbbbb", false],
        ]);
    });

    it("says when no imported deposit can pay it, and keeps Apply payment off", async () => {
        server({...OPTIONS, candidates: []});
        render(ApplyPaymentDialog, {number: "207", onApplied: vi.fn(), onCancel: vi.fn()});
        expect((await screen.findByTestId("apply-payment-none")).textContent).toContain("Import or sync the bank in Ledgeline");
        expect((screen.getByTestId("apply-payment-confirm") as HTMLButtonElement).disabled).toBe(true);
    });

    it("shows a refusal in plain words and stays open", async () => {
        vi.stubGlobal(
            "fetch",
            vi.fn(async () => new Response(JSON.stringify({error: "these are accrual books: link it in Ledgeline", code: "refused"}), {status: 422}))
        );
        const onApplied = vi.fn();
        render(ApplyPaymentDialog, {number: "105", onApplied, onCancel: vi.fn()});
        expect((await screen.findByTestId("apply-payment-load-error")).textContent).toBe("These are accrual books: link it in Ledgeline");
        expect(onApplied).not.toHaveBeenCalled();
    });

    it("cancels on Escape", async () => {
        server();
        const onCancel = vi.fn();
        render(ApplyPaymentDialog, {number: "207", onApplied: vi.fn(), onCancel});
        document.body.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
        expect(onCancel).toHaveBeenCalledOnce();
    });
});
