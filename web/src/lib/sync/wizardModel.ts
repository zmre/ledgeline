// The Connect-a-bank wizard's logic (plan 43 §4.3): its steps, the setup
// token check that runs as the user pastes, and each account's draft link.
//
// Pure, so the step machine, every token message and every draft rule are
// unit-tested. Relative imports only. Nothing here talks to the server: the
// component sends what `toLinkRequest` builds, and feeds the answers back in
// as events.

import {daysBetween, formatMoney, negate, shortDate} from "./format";
import type {NewAccountType, WireLinkPreview, WireLinkRequest, WireRemoteAccount, WireRulesChoice, WireSyncRun} from "./types";

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

export type WizardStep = "choose" | "simplefin" | "link" | "preview" | "done";

/** The rail, in order. "done" is the screen after Finish and has no rail entry. */
export const STEP_ORDER: readonly WizardStep[] = ["choose", "simplefin", "link", "preview"];

export const STEP_LABELS: Record<WizardStep, string> = {
    choose: "How to connect",
    simplefin: "Set up SimpleFIN",
    link: "Link accounts",
    preview: "First sync",
    done: "Done",
};

export type ConnectMethod = "simplefin" | "manual";

export interface WizardState {
    step: WizardStep;
    /** Where this wizard opened. Back never goes before it: "Replace token…" starts at simplefin, "Link…" at link. */
    start: WizardStep;
    /** Set once a token is claimed, or given when the wizard opens on an existing connection. */
    connectionId: string | null;
    /** What SimpleFIN listed, for the link step. */
    accounts: readonly WireRemoteAccount[];
    previews: readonly WireLinkPreview[] | null;
    run: WireSyncRun | null;
    /** Closed — by "I'll download statements myself", which also asks the Accounts tab to highlight Manual downloads. */
    closed: null | {highlightManual: boolean};
}

export type WizardEvent =
    | {type: "choose"; method: ConnectMethod}
    | {type: "claimed"; connectionId: string; accounts: readonly WireRemoteAccount[]}
    /** A re-check found these (possibly none). */
    | {type: "accounts"; accounts: readonly WireRemoteAccount[]}
    | {type: "previewed"; previews: readonly WireLinkPreview[]}
    | {type: "finished"; run: WireSyncRun}
    | {type: "back"};

export function wizardInit(start: WizardStep = "choose", connectionId: string | null = null, accounts: readonly WireRemoteAccount[] = []): WizardState {
    return {step: start, start, connectionId, accounts, previews: null, run: null, closed: null};
}

/** The previous step, or null at the wizard's own first step (and after Finish: nothing to go back to). */
export function previousStep(state: WizardState): WizardStep | null {
    if (state.step === "done" || state.step === state.start) return null;
    const at = STEP_ORDER.indexOf(state.step);
    return at <= 0 ? null : STEP_ORDER[at - 1];
}

/** The steps the rail shows: from where the wizard opened to the last. */
export function railSteps(state: WizardState): readonly WizardStep[] {
    const from = STEP_ORDER.indexOf(state.start === "done" ? "preview" : state.start);
    return STEP_ORDER.slice(Math.max(0, from));
}

export function wizardReduce(state: WizardState, event: WizardEvent): WizardState {
    switch (event.type) {
        case "choose":
            if (state.step !== "choose") return state;
            return event.method === "manual" ? {...state, closed: {highlightManual: true}} : {...state, step: "simplefin"};
        case "claimed":
            return {...state, step: "link", connectionId: event.connectionId, accounts: event.accounts, previews: null};
        case "accounts":
            return {...state, accounts: event.accounts};
        case "previewed":
            return state.step === "link" ? {...state, step: "preview", previews: event.previews} : state;
        case "finished":
            return state.step === "preview" ? {...state, step: "done", run: event.run} : state;
        case "back": {
            const prev = previousStep(state);
            if (prev === null) return state;
            // Leaving the preview drops it: it describes drafts that are about to change.
            return {...state, step: prev, previews: null};
        }
    }
}

// ---------------------------------------------------------------------------
// The setup token (plan 43 §4.3 step 2, "Live validation")
// ---------------------------------------------------------------------------

/** The hosts that ARE SimpleFIN Bridge. Any other is allowed, with a warning: other SimpleFIN servers exist. */
export const BRIDGE_HOSTS: readonly string[] = ["beta-bridge.simplefin.org", "bridge.simplefin.org"];

export type TokenCheck = {state: "empty"} | {state: "invalid"; message: string} | {state: "ok"; host: string; claimUrl: string; bridge: boolean; note: string};

const NOT_A_TOKEN = "That isn't a setup token. Copy the whole token SimpleFIN shows you.";

/** Standard or URL-safe base64, padded or not, as UTF-8 text; null when it is not. */
export function decodeBase64(input: string): string | null {
    const compact = input.replace(/\s+/g, "");
    if (compact === "" || !/^[A-Za-z0-9+/_-]+={0,2}$/.test(compact)) return null;
    const standard = compact.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
    if (standard.length % 4 === 1) return null;
    const padded = standard + "=".repeat((4 - (standard.length % 4)) % 4);
    try {
        const binary = atob(padded);
        const bytes = Uint8Array.from(binary, (ch) => ch.charCodeAt(0));
        return new TextDecoder("utf-8", {fatal: true}).decode(bytes);
    } catch {
        return null;
    }
}

/**
 * Check a pasted setup token in the browser, before anything is sent.
 *
 * A setup token is base64 of an `https://…/claim/…` URL. The browser decodes
 * it to show whose server it names — the one fact a person can check — and
 * refuses plain `http`, which would send the claim in the clear. The server
 * checks all of it again; this is for the person, not for security.
 */
export function checkSetupToken(input: string): TokenCheck {
    const trimmed = input.trim();
    if (trimmed === "") return {state: "empty"};
    if (/^https?:\/\/[^/]*@/i.test(trimmed)) {
        return {state: "invalid", message: "That's an access URL, not a setup token. Paste the setup token SimpleFIN showed you."};
    }
    const decoded = decodeBase64(trimmed);
    if (decoded === null) return {state: "invalid", message: NOT_A_TOKEN};
    let url: URL;
    try {
        url = new URL(decoded.trim());
    } catch {
        return {state: "invalid", message: NOT_A_TOKEN};
    }
    if (url.protocol === "http:") {
        return {state: "invalid", message: `This token points to ${url.host} over plain http, which isn't safe. Ledgeline only connects over https.`};
    }
    if (url.protocol !== "https:" || !url.pathname.includes("/claim/")) return {state: "invalid", message: NOT_A_TOKEN};
    const host = url.host.toLowerCase();
    const bridge = BRIDGE_HOSTS.includes(host);
    return {
        state: "ok",
        host,
        claimUrl: url.href,
        bridge,
        note: bridge ? `Token from ${host}` : `Not SimpleFIN Bridge. Only continue if you trust ${host}.`,
    };
}

/** Whether clipboard text is worth offering as a token: it decodes to a claim URL (http included, so the refusal is shown). */
export function looksLikeSetupToken(text: string): boolean {
    const check = checkSetupToken(text);
    return check.state === "ok" || (check.state === "invalid" && check.message.includes("plain http"));
}

// ---------------------------------------------------------------------------
// Link drafts (plan 43 §4.3 step 3)
// ---------------------------------------------------------------------------

/** How the categories question was answered when a manual rules file already covers the account. */
export type ShareChoice = "ask" | "share" | "separate";

export interface LinkDraft {
    remoteId: string;
    /** False: "Don't sync". */
    include: boolean;
    /** True: create `newAccount.name` instead of mapping onto an existing account. */
    createAccount: boolean;
    journalAccount: string;
    newAccount: {name: string; type: NewAccountType; openingBalance: string; openingDate: string};
    startDate: string;
    target: string;
    /** `"new"` for a generated file, else an existing sync rules file's id. */
    rulesChoice: "new" | string;
    newRulesName: string;
    share: ShareChoice;
    balanceSign: 1 | -1;
}

/** Generic words that say nothing about WHICH account it is. */
const GENERIC = new Set(["bank", "banks", "cc", "credit", "card", "cards", "creditcard", "creditcards", "accounts", "account"]);

/**
 * The name for a new sync rules file, from the journal account:
 * `assets:bank:firsttech:checking` → `firsttech-checking.sync.csv.rules`,
 * `liabilities:cc:chase` → `chase.sync.csv.rules`. The type root and the
 * generic words go; what is left names the account.
 */
export function newRulesName(journalAccount: string): string {
    const words = journalAccount
        .split(":")
        .slice(1)
        .map((w) =>
            w
                .toLowerCase()
                .replace(/[^a-z0-9]+/g, "-")
                .replace(/^-+|-+$/g, "")
        )
        .filter((w) => w !== "" && !GENERIC.has(w));
    return `${words.length === 0 ? "sync" : words.join("-")}.sync.csv.rules`;
}

/**
 * A remote account's draft, seeded from the server's suggestion. An account
 * with no suggested journal account starts as "Don't sync": nothing is linked
 * by accident.
 */
export function draftFor(remote: WireRemoteAccount, defaultTarget: string, today: string): LinkDraft {
    const s = remote.suggestion;
    const journalAccount = s.journalAccount ?? "";
    return {
        remoteId: remote.remoteId,
        include: s.journalAccount !== null,
        createAccount: false,
        journalAccount,
        newAccount: {
            name: s.newAccountName,
            type: s.newAccountType,
            openingBalance: remote.balance,
            openingDate: remote.balanceDate ?? today,
        },
        startDate: s.startDate,
        target: s.target ?? defaultTarget,
        rulesChoice: rulesFor(s.syncRules, journalAccount === "" ? s.newAccountName : journalAccount),
        newRulesName: newRulesName(journalAccount === "" ? s.newAccountName : journalAccount),
        share: s.manualRules === null ? "separate" : "ask",
        balanceSign: s.signFlipped ? -1 : 1,
    };
}

/** The journal account the draft writes to, whichever way it was chosen. */
export const draftAccount = (draft: LinkDraft): string => (draft.createAccount ? draft.newAccount.name.trim() : draft.journalAccount.trim());

/**
 * The rules choice for `account`: an existing sync rules file whose
 * `account1` is that account, else `"new"`. Another account's file is listed
 * but never preselected.
 */
export function rulesFor(choices: readonly WireRulesChoice[], account: string): "new" | string {
    const mine = choices.find((c) => c.fits && c.account1 !== null && c.account1 === account.trim());
    return mine?.id ?? "new";
}

/**
 * Follow a change of the draft's journal account (picked, typed as a new
 * account, or by switching between the two): the new file's name follows it,
 * and the choice moves to that account's own file, or to the new one. A file
 * picked by hand stays picked only while it belongs to no account in
 * particular.
 */
export function followAccount(draft: LinkDraft, choices: readonly WireRulesChoice[]): void {
    // An empty existing-account box still names the file after the suggestion.
    const account = draftAccount(draft) || draft.newAccount.name.trim();
    draft.newRulesName = newRulesName(account);
    const picked = choices.find((c) => c.id === draft.rulesChoice);
    if (draft.rulesChoice === "new" || picked === undefined || (picked.account1 !== null && picked.account1 !== account)) {
        draft.rulesChoice = rulesFor(choices, account);
    }
}

const ISO = /^\d{4}-\d{2}-\d{2}$/;
const DECIMAL = /^[+-]?\d+(\.\d+)?$/;

/** What stops this draft from being linked, in the order a person would fix it. Empty for a draft that is skipped. */
export function draftIssues(draft: LinkDraft): string[] {
    if (!draft.include) return [];
    const issues: string[] = [];
    const account = draftAccount(draft);
    if (account === "") issues.push("Choose a journal account.");
    else if (!/^[^\s:][^:]*(:[^\s:][^:]*)+$/.test(account) || /\s{2}|\t/.test(account)) issues.push("A journal account looks like assets:bank:checking.");
    if (draft.createAccount && !DECIMAL.test(draft.newAccount.openingBalance.trim())) issues.push("The opening balance must be a number, like 1234.56.");
    if (draft.createAccount && !ISO.test(draft.newAccount.openingDate)) issues.push("Choose the opening balance's date.");
    if (!ISO.test(draft.startDate)) issues.push("Choose a date to start syncing from.");
    if (draft.target.trim() === "") issues.push("Choose the file to write into.");
    if (draft.rulesChoice === "new" && !/^[A-Za-z0-9][A-Za-z0-9._-]*\.rules$/.test(draft.newRulesName.trim())) {
        issues.push("A new rules file needs a plain name ending in .rules.");
    }
    if (draft.share === "ask") issues.push("Say whether to share the existing categorization rules.");
    return issues;
}

/** The request for one draft, or null for "Don't sync". */
export function toLinkRequest(draft: LinkDraft, remote: WireRemoteAccount): WireLinkRequest | null {
    if (!draft.include) return null;
    const manual = remote.suggestion.manualRules;
    return {
        connectionId: remote.connectionId,
        remoteId: remote.remoteId,
        label: remote.label,
        journalAccount: draftAccount(draft),
        newAccount: draft.createAccount
            ? {type: draft.newAccount.type, openingBalance: draft.newAccount.openingBalance.trim(), openingDate: draft.newAccount.openingDate}
            : null,
        startDate: draft.startDate,
        target: draft.target.trim(),
        balanceSign: draft.balanceSign,
        rules: draft.rulesChoice === "new" ? {mode: "new", name: draft.newRulesName.trim()} : {mode: "existing", id: draft.rulesChoice},
        share: draft.share === "share" && manual !== null ? {manualRulesId: manual.id} : null,
    };
}

/** Requests for every included draft, or the issues that block them (prefixed with the account's label). */
export function linkRequests(drafts: readonly LinkDraft[], remotes: readonly WireRemoteAccount[]): {requests: WireLinkRequest[]; issues: string[]} {
    const byId = new Map(remotes.map((r) => [r.remoteId, r]));
    const requests: WireLinkRequest[] = [];
    const issues: string[] = [];
    for (const draft of drafts) {
        const remote = byId.get(draft.remoteId);
        if (remote === undefined) continue;
        const problems = draftIssues(draft);
        if (problems.length > 0) issues.push(...problems.map((p) => `${remote.label}: ${p}`));
        const request = toLinkRequest(draft, remote);
        if (request !== null) requests.push(request);
    }
    if (requests.length === 0 && issues.length === 0) issues.push("Choose at least one account to sync.");
    return {requests, issues};
}

/**
 * The one-line sign question (§4.3 "Balance sign check"), or null when the
 * signs agree: "Chase reports this as +1,032.18. Your ledger has −1,032.18.
 * Treat Chase's sign as reversed?"
 */
export function signQuestion(remote: WireRemoteAccount): string | null {
    const s = remote.suggestion;
    if (!s.signFlipped || s.ledgerBalance === null) return null;
    const who = remote.org ?? "The bank";
    const signed = (text: string): string => {
        const shown = formatMoney(text, "");
        return text.trim().startsWith("-") ? shown : `+${shown}`;
    };
    return `${who} reports this as ${signed(remote.balance)}. Your ledger has ${signed(s.ledgerBalance)}. Treat ${remote.org ?? "the bank"}'s sign as reversed?`;
}

/** How the bank's balance compares with the ledger's suggestion, for the line beside the account picker. */
export function balanceCompare(remote: WireRemoteAccount, sign: 1 | -1): {text: string; matches: boolean} | null {
    const ledger = remote.suggestion.ledgerBalance;
    if (ledger === null) return null;
    const bank = sign === -1 ? negate(remote.balance) : remote.balance;
    const matches = Number(bank) === Number(ledger);
    const close = !matches && Math.abs(Number(bank) - Number(ledger)) <= Math.max(1, Math.abs(Number(ledger)) * 0.02);
    return {text: `ledger: ${formatMoney(ledger, remote.currency)}${matches ? " ✓ matches" : close ? " ✓ close" : ""}`, matches: matches || close};
}

/** SimpleFIN serves at most 90 days a request; the wizard warns when the history asked for needs more than 4. */
export const DAYS_PER_REQUEST = 90;

/** The warning for a start date that will cost several requests, or null. */
export function chunkNote(startDate: string, today: string): string | null {
    const days = daysBetween(startDate, today);
    if (days === null || days <= 0) return null;
    const requests = Math.ceil((days + 1) / DAYS_PER_REQUEST);
    if (requests <= 4) return null;
    return `That much history takes ${requests} requests to SimpleFIN, which allows about 24 a day.`;
}

/** "First National only provides history back to Aug 20", when the start date is earlier than anything the feed sent. */
export function historyNote(remote: WireRemoteAccount, startDate: string, today: string, locale?: string): string | null {
    const oldest = remote.oldestAvailable;
    if (oldest === null || !ISO.test(startDate) || startDate >= oldest) return null;
    return `${remote.org ?? remote.label} only provides history back to ${shortDate(oldest, today, locale)}.`;
}

/** "38 new · 0 already in your journal · 2 possible duplicates held · 6 uncategorized" */
export function previewLine(preview: WireLinkPreview): string {
    const parts = [`${preview.new} new`, `${preview.already} already in your journal`];
    parts.push(preview.held === 1 ? "1 possible duplicate held" : `${preview.held} possible duplicates held`);
    parts.push(`${preview.uncategorized} uncategorized`);
    if (preview.droppedBeforeStart > 0) parts.push(`${preview.droppedBeforeStart} before the start date`);
    return parts.join(" · ");
}
