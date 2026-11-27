// The note check every time form makes (Track, Entries, billing), as the
// server's `note_problem` does.
import {describe, expect, it} from "vitest";
import {noteProblem} from "./note.js";

describe("UNIT the note check, as the server makes it", () => {
    it("lets plain notes through", () => {
        expect(noteProblem("")).toBeNull();
        expect(noteProblem("call about the K-1")).toBeNull();
        expect(noteProblem("drafts, emails and a call")).toBeNull();
    });

    it("names the word hledger would read as a tag, with the server's rewrite", () => {
        // The server's own case (time_tests.rs).
        expect(noteProblem("call re: K-1")).toMatchObject({offending: "re:", suggestion: "call re K-1"});
        expect(noteProblem("call at 10:30")).toMatchObject({offending: "10:", suggestion: "call at 10.30"});
        expect(noteProblem("ok, then todo: file it")).toMatchObject({offending: "todo:", suggestion: "ok, then todo file it"});
    });

    it("refuses what would end the note early", () => {
        expect(noteProblem("call; then email")).toMatchObject({offending: ";", suggestion: "call, then email"});
        expect(noteProblem("two\nlines")).toMatchObject({offending: "a line break", suggestion: "two lines"});
    });

    it("finds a tag wherever it sits, as Entries did", () => {
        expect(noteProblem("call with Fred")).toBeNull();
        expect(noteProblem("re: K-1")?.offending).toBe("re:");
        expect(noteProblem("see https://example.com")?.offending).toBe("https:");
        expect(noteProblem("call re: K-1")?.message).toContain("“re:” would be read as a tag");
    });
});
