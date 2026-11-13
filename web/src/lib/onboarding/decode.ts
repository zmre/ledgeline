// File → New wizard (plan 39 Phase 6): decoders for the `/api/new-ledger*`
// bodies. Kept beside the wizard rather than in `nativeDecode.ts` because no
// other screen reads them; same conventions (throw `ApiShapeError`, freeze).

import {ApiShapeError} from "#lib/api/client.js";
import {bool, str} from "#lib/api/journalViewDecode.js";
import type {AccountTag, CreatedLedger, NewLedgerStatus, PickedFolder, TemplateAccount} from "./types";

function object(raw: unknown, context: string): Record<string, unknown> {
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an object`);
    return raw as Record<string, unknown>;
}

function array(raw: unknown, context: string): unknown[] {
    if (!Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an array`);
    return raw;
}

/** `GET /api/new-ledger`. */
export function decodeNewLedgerStatus(raw: unknown): NewLedgerStatus {
    const body = object(raw, "new-ledger status");
    return Object.freeze({desktop: bool(body.desktop, "new-ledger desktop"), journalOpen: bool(body.journalOpen, "new-ledger journalOpen")});
}

function decodeTag(raw: unknown, context: string): AccountTag {
    const tag = object(raw, context);
    return Object.freeze({name: str(tag.name, `${context} name`), value: str(tag.value, `${context} value`)});
}

function decodeAccount(raw: unknown, context: string): TemplateAccount {
    const account = object(raw, context);
    return Object.freeze({
        name: str(account.name, `${context} name`),
        tags: Object.freeze(array(account.tags, `${context} tags`).map((tag, i) => decodeTag(tag, `${context} tags[${i}]`))),
    });
}

/** `GET /api/new-ledger/template` → the chart's accounts, in file order. */
export function decodeTemplate(raw: unknown): readonly TemplateAccount[] {
    const body = object(raw, "new-ledger template");
    return Object.freeze(array(body.accounts, "new-ledger template accounts").map((account, i) => decodeAccount(account, `template accounts[${i}]`)));
}

/** `POST /api/new-ledger/folder` → the picked folder, or null when cancelled. */
export function decodePickedFolder(raw: unknown): PickedFolder | null {
    const body = object(raw, "new-ledger folder");
    if (body.folder === null) return null;
    const folder = object(body.folder, "new-ledger folder");
    return Object.freeze({
        id: str(folder.id, "new-ledger folder id"),
        name: str(folder.name, "new-ledger folder name"),
        empty: bool(folder.empty, "new-ledger folder empty"),
    });
}

/** `POST /api/new-ledger` and `/sample` → what was written. */
export function decodeCreatedLedger(raw: unknown): CreatedLedger {
    const body = object(raw, "new ledger");
    return Object.freeze({
        folder: str(body.folder, "new ledger folder"),
        files: Object.freeze(array(body.files, "new ledger files").map((file, i) => str(file, `new ledger files[${i}]`))),
    });
}
