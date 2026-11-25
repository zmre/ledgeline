// A list's search box (invoicer PLAN 5b): its own text, sent after a pause.
// The URL is the list's state, so what was sent comes back as the URL's `q`;
// `sync` takes that back without overwriting what the user has typed since,
// while a different `q` (Back, a link) replaces the text.

export class SearchBox {
    /** What the box shows. */
    text = $state("");
    #sent: string;
    #timer: ReturnType<typeof setTimeout> | null = null;
    readonly #send: (q: string) => void;
    readonly #delay: number;

    /** `send` gets the trimmed text after `delay` ms without typing, or at once on `flush`. */
    constructor(initial: string, send: (q: string) => void, delay = 250) {
        this.text = initial;
        this.#sent = initial.trim();
        this.#send = send;
        this.#delay = delay;
    }

    /** The user typed `text`: send it after the pause. */
    type(text: string): void {
        this.text = text;
        this.#cancel();
        this.#timer = setTimeout(() => this.flush(), this.#delay);
    }

    /** Send now (Enter, leaving the box) if it differs from what was sent. */
    flush(): void {
        this.#cancel();
        const q = this.text.trim();
        if (q === this.#sent) return;
        this.#sent = q;
        this.#send(q);
    }

    /** The URL's `q` now: a value this box didn't send replaces its text. */
    sync(q: string): void {
        if (q.trim() === this.#sent) return;
        this.#cancel();
        this.text = q;
        this.#sent = q.trim();
    }

    /** Stop a pending send (the page is going away). */
    dispose(): void {
        this.#cancel();
    }

    #cancel(): void {
        if (this.#timer !== null) clearTimeout(this.#timer);
        this.#timer = null;
    }
}
