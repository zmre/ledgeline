// /time/unbilled, To bill's old address: it redirects to /to-bill, the query
// kept, taking up a launch token first (the redirect drops the fragment).
import {isRedirect} from "@sveltejs/kit";
import {afterEach, describe, expect, it, vi} from "vitest";
import {LAUNCH_TOKEN_STORAGE_KEY} from "#ui/api/client.js";

vi.mock("$app/paths", () => ({resolve: (path: string) => `/app${path}`}));

import {load} from "./+page.js";

function where(url: string): string {
    try {
        void load({url: new URL(url)} as Parameters<typeof load>[0]);
    } catch (e: unknown) {
        if (isRedirect(e)) return e.location;
        throw e;
    }
    throw new Error("no redirect");
}

describe("COMPONENT the old Unbilled address", () => {
    afterEach(() => sessionStorage.removeItem(LAUNCH_TOKEN_STORAGE_KEY));

    it("redirects to To bill, keeping the query", () => {
        expect(where("http://localhost/app/time/unbilled")).toBe("/app/to-bill");
        expect(where("http://localhost/app/time/unbilled?tab=pay")).toBe("/app/to-bill?tab=pay");
    });

    it("keeps a launch token the redirect would drop", () => {
        history.replaceState(null, "", "/app/time/unbilled#token=abc");
        expect(where("http://localhost/app/time/unbilled")).toBe("/app/to-bill");
        expect(sessionStorage.getItem(LAUNCH_TOKEN_STORAGE_KEY)).toBe("abc");
        expect(location.hash).toBe("");
    });
});
