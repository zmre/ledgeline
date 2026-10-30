// The edit dialog's DRAFT (plan 31 C3, C11): a deep copy of one entry, edited
// in the dialog, applied to the scenario only on Done.
//
// Nothing in `scenarioStore.scenario` changes while the dialog is open, so no
// run fires and the readings below the list stay as they were. Done applies the
// whole draft as ONE new scenario (`applyDraft`), which the dialog hands to
// `scenarioStore.commit` — one revision, one debounced run. Cancel, Escape and
// the backdrop just drop the draft, which is why "Cancel on a fresh duplicate
// leaves no copy" and "Cancel on Duplicate as alternative leaves the original
// enabled" hold by construction: neither the copy nor the turned-off original
// ever existed outside the draft.
//
// The helpers are pure. `EntryPreview` is the one rune class: the "Writes:"
// box's debounced call to `POST /api/projections/render`, which drops stale
// answers so an older request resolving last never overwrites a newer one.

import {ValidationError} from "#lib/api/native.js";
import {ruleFormComplete} from "#lib/autoRules/ruleForm.js";
import type {ISODate} from "#lib/domain/types.js";
import {
    attachedDisplay,
    blankPayload,
    canDisable,
    clonePayload,
    copyEntry,
    insertEntry,
    newItem,
    payloadKey,
    payloadOf,
    plainCopy,
    replacePayload,
    setDisabled,
    splitBlock,
    type AddKind,
    type BlankContext,
    type Payload,
} from "./entries";
import {isIsoDate, layoutItemToWire, noteProblem, renderBody, withLayout} from "./scenarioModel";
import type {LayoutItem, Scenario} from "./types";
import type {RenderProjectionBody} from "#lib/api/native.js";

/** How a new entry came to be, for the dialog's title and the list's placeholder. */
export type DraftOrigin = "edit" | "add" | "duplicate" | "alternative" | "step";

export interface Draft {
    /** The entry's key: its own, or the new entry's. */
    key: string;
    origin: DraftOrigin;
    /** The entry's content: a deep copy, so mutating it never reaches the scenario. */
    payload: Payload;
    /** Its layout item, copied. `disabled` is the dialog's Enabled toggle (inverted). */
    item: LayoutItem;
    /** "Comment above this entry", as display text without `; `. */
    attached: string;
    /** For a new entry: the key it is inserted directly after, or null for the end. */
    after: string | null;
    /** Duplicate as alternative: the original, turned off on Done. */
    disableOriginal: string | null;
    /** Add a step change: the original rule's lines, bounded at the split, written on Done. */
    stepBefore: Payload | null;
    /** Whether anything has been edited. An untouched edit's Done changes nothing. */
    edited: boolean;
}

/** A draft of the existing entry `key`, or null when there is nothing editable there (file text). */
export function draftOf(full: Scenario, key: string): Draft | null {
    const scenario = withLayout(full);
    const item = scenario.layout.find((candidate) => candidate.key === key);
    const payload = payloadOf(scenario, key);
    if (item === undefined || payload === null) return null;
    return {
        key,
        origin: "edit",
        payload: clonePayload(payload),
        item: plainCopy(item),
        attached: item.attachedText ?? attachedDisplay(item.attached),
        after: null,
        disableOriginal: null,
        stepBefore: null,
        edited: false,
    };
}

/** A draft of a brand-new entry, inserted after `after` on Done. Always counts as edited: Done must add it. */
function newDraft(payload: Payload, item: LayoutItem, after: string | null, origin: DraftOrigin, attached: string): Draft {
    return {key: payloadKey(payload), origin, payload, item, attached, after, disableOriginal: null, stepBefore: null, edited: true};
}

/** Add: a blank entry of `kind`, after `after` (the cursor's entry) or at the end. */
export function addDraft(scenario: Scenario, kind: AddKind, after: string | null, context: BlankContext): Draft {
    const payload = blankPayload(withLayout(scenario), kind, context);
    return newDraft(payload, newItem(payload.kind, payloadKey(payload)), after, "add", "");
}

/**
 * Duplicate (or, with `alternative`, Duplicate as alternative): a copy of
 * `key` under fresh keys, directly below it. The alternative also records the
 * original, which Done turns off in the same mutation.
 */
export function duplicateDraft(scenario: Scenario, key: string, alternative: boolean): Draft | null {
    const copy = copyEntry(scenario, key);
    if (copy === null) return null;
    const draft = newDraft(
        copy.payload,
        copy.item,
        key,
        alternative ? "alternative" : "duplicate",
        copy.item.attachedText ?? attachedDisplay(copy.item.attached)
    );
    return alternative ? {...draft, disableOriginal: key} : draft;
}

/** Add a step change: the later segment of `group` split at `on`, directly below; null when it cannot split there. */
export function stepDraft(scenario: Scenario, group: string, on: ISODate): Draft | null {
    const split = splitBlock(scenario, group, on);
    if (split === null) return null;
    const payload: Payload = {kind: "rule", lines: split.after};
    return {...newDraft(payload, newItem("rule", payloadKey(payload)), group, "step", ""), stepBefore: {kind: "rule", lines: split.before}};
}

/**
 * `scenario` with `draft` applied: the edited entry replaced in place, or the
 * new one inserted at its point — plus, for an alternative, the original
 * turned off, and for a step, the original bounded at the split. ONE new
 * scenario, so the store sees one edit.
 *
 * `rendered` is the text the dialog's preview showed for this draft. It
 * becomes the item's `raw`, which is what the card's "rule text" disclosure
 * and the next dialog's "Writes:" show until that entry is edited again —
 * exact, because a save renders the entry with the same functions.
 */
export function applyDraft(full: Scenario, draft: Draft, rendered: string | null = null): Scenario {
    const scenario = withLayout(full);
    const payload = clonePayload(draft.payload);
    const original = scenario.layout.find((item) => item.key === draft.key);
    const item = itemOf(draft, original, rendered);
    if (draft.origin === "edit") {
        const replaced = replacePayload(scenario, draft.key, payload);
        return {...replaced, layout: replaced.layout.map((candidate) => (candidate.key === draft.key ? item : candidate))};
    }
    let next = draft.stepBefore === null || draft.after === null ? scenario : replacePayload(scenario, draft.after, clonePayload(draft.stepBefore));
    next = insertEntry(next, payload, draft.after, item);
    if (draft.disableOriginal !== null) next = setDisabled(next, draft.disableOriginal, true);
    return next;
}

/** The layout item a draft commits: its toggle, its attached comment as edited text when it changed, and its new `raw`. */
function itemOf(draft: Draft, original: LayoutItem | undefined, rendered: string | null): LayoutItem {
    const base = plainCopy(draft.item);
    delete base.attachedText;
    const fileText = attachedDisplay(base.attached);
    const attached = !canDisable(base.kind) || draft.attached === fileText ? {} : {attachedText: draft.attached};
    const raw = rendered ?? original?.raw ?? base.raw;
    return {...base, ...attached, raw};
}

/** The layout item a render is asked for: the draft's toggle and attached comment. */
export function draftWireItem(draft: Draft) {
    return layoutItemToWire(itemOf(draft, undefined, null));
}

/** The `POST /api/projections/render` body for a draft: the scenario with the draft applied, and the draft's own item. */
export function draftRenderBody(scenario: Scenario, draft: Draft): RenderProjectionBody {
    return renderBody(applyDraft(scenario, draft), draft.key, draftWireItem(draft));
}

/**
 * What stops Done, in words, or `[]`. The checks a run would otherwise make
 * by dropping the entry (`projectable`), and the description rules the writer
 * refuses (`noteProblem`); everything else is the server's, through the
 * preview's refusal.
 */
export function draftProblems(draft: Draft): string[] {
    const problems: string[] = [];
    const payload = draft.payload;
    switch (payload.kind) {
        case "rule": {
            if (payload.lines.some((line) => line.account.trim() === "")) problems.push("Every posting needs an account.");
            const note = noteProblem(payload.lines[0]?.note ?? "");
            if (note !== null) problems.push(note);
            if ((payload.lines[0]?.period.raw.trim() ?? "") === "") problems.push("The rule needs a period.");
            break;
        }
        case "event": {
            if (!isIsoDate(payload.event.date)) problems.push("The one-off needs a date.");
            if (payload.event.postings.length === 0 || payload.event.postings.some((posting) => posting.account.trim() === "")) {
                problems.push("Every posting needs an account.");
            }
            const note = noteProblem(payload.event.description);
            if (note !== null) problems.push(note);
            break;
        }
        case "auto":
            if (payload.rule.form !== null && (payload.rule.edited === true || payload.rule.raw === "") && !ruleFormComplete(payload.rule.form)) {
                problems.push("Name the matched account, and an account and a number for every posting.");
            }
            break;
        case "comment":
            if (payload.comment.text.trim() === "")
                problems.push(payload.comment.style === "section" ? "The heading needs a title." : "The note needs some text.");
            break;
    }
    return problems;
}

// ---------------------------------------------------------------------------
// The "Writes:" preview
// ---------------------------------------------------------------------------

/** The render debounce: one request per 300ms of editing (plan 31 C3). */
export const PREVIEW_DEBOUNCE_MS = 300;

/**
 * The dialog's exact preview of what a save writes.
 *
 * `show(raw)` puts the piece's own text up (an unchanged piece is written
 * with its own bytes). `request(body)` asks the server, debounced: while a
 * request is pending the previous text stays (the dialog dims it), a refusal
 * puts the writer's own sentence in `refusal`, and a response that a newer
 * request has superseded is ignored.
 */
export class EntryPreview {
    /** The text on screen. */
    text = $state("");
    /** The writer's sentence for a refused draft (a 400), or null. Done stays disabled while it is set. */
    refusal = $state<string | null>(null);
    /** Any other failure (the engine unreachable, too old): said, but it does not block Done. */
    failure = $state<string | null>(null);
    /** A request is scheduled or in flight. */
    pending = $state(false);
    /** The text is the server's render of the latest request (not the piece's `raw`, not a stale answer). */
    rendered = $state(false);

    #render: (body: RenderProjectionBody) => Promise<string>;
    #delay: number;
    #timer: ReturnType<typeof setTimeout> | null = null;
    #seq = 0;

    constructor(render: (body: RenderProjectionBody) => Promise<string>, delay = PREVIEW_DEBOUNCE_MS) {
        this.#render = render;
        this.#delay = delay;
    }

    /** Show a piece's own text, as it stands in the file. */
    show(raw: string): void {
        this.stop();
        this.text = raw;
        this.refusal = null;
        this.failure = null;
        this.rendered = false;
    }

    /** Ask for `body`'s render after the debounce; a newer call replaces this one. */
    request(body: RenderProjectionBody): void {
        if (this.#timer !== null) clearTimeout(this.#timer);
        this.pending = true;
        this.rendered = false;
        this.#timer = setTimeout(() => {
            this.#timer = null;
            void this.#run(body);
        }, this.#delay);
    }

    /** Hold the preview where it is: nothing to render (the draft has problems of its own). */
    hold(): void {
        this.stop();
        this.rendered = false;
    }

    /** Cancel a scheduled request, and forget any in flight. */
    stop(): void {
        if (this.#timer !== null) clearTimeout(this.#timer);
        this.#timer = null;
        this.#seq += 1;
        this.pending = false;
    }

    async #run(body: RenderProjectionBody): Promise<void> {
        this.#seq += 1;
        const seq = this.#seq;
        try {
            const text = await this.#render(body);
            if (seq !== this.#seq) return;
            this.text = text;
            this.refusal = null;
            this.failure = null;
            this.rendered = true;
        } catch (error) {
            if (seq !== this.#seq) return;
            const message = error instanceof Error ? error.message : String(error);
            if (error instanceof ValidationError) {
                this.refusal = message;
                this.failure = null;
            } else {
                this.refusal = null;
                this.failure = message;
            }
        } finally {
            if (seq === this.#seq) this.pending = false;
        }
    }
}
