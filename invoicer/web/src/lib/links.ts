// Wikilinks without brackets for the user (invoicer PLAN, Phase 4 feedback
// H2): a field that links a note (the customer, Deposit for note) shows the
// note's name and writes `[[Name]]` into the file. The brackets are the
// file's business, never typed.

/**
 * The note a `[[Target]]` / `[[Target|shown]]` / `[[Target#Heading]]` link
 * names; other text as it is (trimmed). A link naming no note (`[[]]`,
 * `[[#Heading]]`) is "": the pages never show `[[…]]`.
 */
export function linkName(text: string | null | undefined): string {
    const t = (text ?? "").trim();
    const inner = /^\[\[([^\]]*)\]\]$/.exec(t)?.[1];
    return inner === undefined ? t : inner.split("|")[0].split("#")[0].trim();
}

/** Whether `text` is a whole `[[wikilink]]`. */
export const isLink = (text: string | null | undefined): boolean => /^\[\[[^\]]+\]\]$/.test((text ?? "").trim());

/**
 * What to write for a name typed or picked: `[[Title]]` when it names one of
 * `titles` (case-insensitively, written as the note spells it); otherwise a
 * link anyway when `always` (the customer must be a link), else the text as
 * typed (free text, as `deposit_for` allows). Blank is "".
 */
export function toLink(name: string, titles: readonly string[], always = false): string {
    const text = name.trim();
    if (text === "") return "";
    if (isLink(text)) return text;
    const known = titles.find((t) => t.toLowerCase() === text.toLowerCase());
    if (known !== undefined) return `[[${known}]]`;
    return always ? `[[${text}]]` : text;
}
