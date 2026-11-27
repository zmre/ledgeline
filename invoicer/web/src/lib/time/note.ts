// The note check the server makes (`note_problem`), mirrored so every form
// that writes a note warns while you type: Track's note field, Entries' add
// and edit, and billing. hledger reads `word:` in a comment as a tag and `;`
// as the end of the note; the server refuses both (`note-tag`) and suggests a
// rewording, which the forms offer. Pure: no runes, no DOM.

/** Why a note can't be written as typed, and a rewording that can. */
export interface NoteProblem {
    /** What hledger would misread: `re:`, `;`, `a line break`. */
    offending: string;
    message: string;
    suggestion: string | null;
}

/** The tag hledger reads in one comma-separated piece: the word before its first colon. */
function tagIn(segment: string): string | null {
    const colon = segment.indexOf(":");
    if (colon === -1) return null;
    const words = segment
        .slice(0, colon)
        .split(/\s+/)
        .filter((w) => w !== "");
    return words.length === 0 ? null : words[words.length - 1];
}

function firstTag(note: string): string | null {
    for (const segment of note.split(",")) {
        const tag = tagIn(segment);
        if (tag !== null) return tag;
    }
    return null;
}

// Control characters are what the server refuses (`char::is_control`).
// eslint-disable-next-line no-control-regex -- matching control characters is the point
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;
// eslint-disable-next-line no-control-regex -- as above
const CONTROL_ALL = /[\u0000-\u001f\u007f-\u009f]/g;

/** The rewording the server suggests: `;` as `,`, `10:30` as `10.30`, other colons dropped. */
function suggestNote(note: string): string | null {
    let out = "";
    for (let at = 0; at < note.length; at += 1) {
        const c = note[at];
        if (c === ";") out += ",";
        else if (c === ":") {
            if (at > 0 && /\d/.test(note[at - 1]) && /\d/.test(note[at + 1] ?? "")) out += ".";
        } else out += c;
    }
    out = out
        .replace(CONTROL_ALL, " ")
        .split(/\s+/)
        .filter((w) => w !== "")
        .join(" ");
    return out !== "" && firstTag(out) === null ? out : null;
}

/**
 * What the server's note check (`note_problem`) would refuse, so Track warns
 * as you type: a `word:` hledger would read as a tag, a `;` or a line break
 * that would end the note early. Null when the note is fine.
 */
export function noteProblem(note: string): NoteProblem | null {
    const breaks = note.includes(";") || CONTROL.test(note);
    const tag = firstTag(note);
    if (!breaks && tag === null) return null;
    const suggestion = suggestNote(note);
    if (breaks) {
        const offending = note.includes(";") ? ";" : "a line break";
        return {offending, message: `A note can't hold ${offending}: it would end the note early.`, suggestion};
    }
    const offending = `${tag}:`;
    return {offending, message: `“${offending}” would be read as a tag, so the note would change meaning.`, suggestion};
}
