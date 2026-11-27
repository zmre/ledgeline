// The window title shows the running timer (plan 46 T-light): `▶ 1h 22m ·
// Invoices · Ledgeline Invoicer`, visible in the Dock, the window menu and
// the tab while the app is in the background. Each page sets its own
// `<title>`, so the prefix is put back whenever a page changes the title.

const MARK = "▶ ";
const SEPARATOR = " · ";

/** `title` without a prefix this put there. */
export function baseTitle(title: string): string {
    if (!title.startsWith(MARK)) return title;
    const at = title.indexOf(SEPARATOR);
    return at === -1 ? title : title.slice(at + SEPARATOR.length);
}

/** `title` with the elapsed time in front, or as it is when `elapsed` is null. */
export function prefixedTitle(title: string, elapsed: string | null): string {
    const base = baseTitle(title);
    return elapsed === null ? base : `${MARK}${elapsed}${SEPARATOR}${base}`;
}

/** Keeps `doc`'s title prefixed with whatever `set` was last given; `stop` restores it. */
export class TitlePrefix {
    #elapsed: string | null = null;
    readonly #doc: Document;
    #observer: MutationObserver | null = null;

    constructor(doc: Document = document) {
        this.#doc = doc;
    }

    set(elapsed: string | null): void {
        this.#elapsed = elapsed;
        this.#apply();
        if (this.#observer === null && typeof MutationObserver !== "undefined") {
            this.#observer = new MutationObserver(() => this.#apply());
            this.#observer.observe(this.#doc.head, {subtree: true, childList: true, characterData: true});
        }
    }

    stop(): void {
        this.#observer?.disconnect();
        this.#observer = null;
        this.#elapsed = null;
        this.#apply();
    }

    #apply(): void {
        const next = prefixedTitle(this.#doc.title, this.#elapsed);
        if (next !== this.#doc.title) this.#doc.title = next;
    }
}
