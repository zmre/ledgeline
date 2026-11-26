import {describe, expect, it} from "vitest";

describe("UNIT api failures", () => {
    it("prefers the server's own sentence to the status line", async () => {
        const {failureMessage} = await import("./api.js");
        const sentence = new Response(JSON.stringify({error: "no books are open; choose File → Open"}), {status: 409});
        expect(await failureMessage("GET", "/api/books", sentence)).toBe("no books are open; choose File → Open");
        const bare = new Response("<html>", {status: 502});
        expect(await failureMessage("GET", "/api/books", bare)).toBe("GET /api/books: HTTP 502");
    });
});
