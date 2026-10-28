// `GET /api/auto-rules` → AutoRulesListing. Hand-mirrored from
// `crates/ledgeline-server/src/auto_api.rs` (`WireAutoRules`), and refused
// loudly when the shape drifts rather than rendering an empty list.

import {ApiShapeError} from "#lib/api/client.js";
import type {AutoRule, AutoRuleProblem, AutoRulesListing} from "./types";

type Raw = Record<string, unknown>;

function obj(raw: unknown, context: string): Raw {
    if (typeof raw !== "object" || raw === null || Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an object`);
    return raw as Raw;
}

function arr(raw: unknown, context: string): unknown[] {
    if (!Array.isArray(raw)) throw new ApiShapeError(`${context}: expected an array`);
    return raw;
}

function str(raw: unknown, context: string): string {
    if (typeof raw !== "string") throw new ApiShapeError(`${context}: expected a string`);
    return raw;
}

function nullableStr(raw: unknown, context: string): string | null {
    return raw === null || raw === undefined ? null : str(raw, context);
}

function count(raw: unknown, context: string): number {
    if (typeof raw !== "number" || !Number.isInteger(raw) || raw < 0) throw new ApiShapeError(`${context}: expected a count`);
    return raw;
}

function bool(raw: unknown, context: string): boolean {
    if (typeof raw !== "boolean") throw new ApiShapeError(`${context}: expected a boolean`);
    return raw;
}

const KINDS: ReadonlySet<string> = new Set<AutoRuleProblem["kind"]>(["refused", "unbalanced", "unknownAccount"]);

function decodeProblem(raw: unknown, context: string): AutoRuleProblem {
    const value = obj(raw, context);
    const kind = str(value.kind, `${context}.kind`);
    if (!KINDS.has(kind)) throw new ApiShapeError(`${context}: unknown problem kind ${JSON.stringify(kind)}`);
    return Object.freeze({
        kind: kind as AutoRuleProblem["kind"],
        message: str(value.message, `${context}.message`),
        txnIndex: value.txnIndex === null || value.txnIndex === undefined ? null : count(value.txnIndex, `${context}.txnIndex`),
        account: nullableStr(value.account, `${context}.account`),
    });
}

function decodeRule(raw: unknown, context: string): AutoRule {
    const value = obj(raw, context);
    return Object.freeze({
        index: count(value.index, `${context}.index`),
        file: str(value.file, `${context}.file`),
        line: count(value.line, `${context}.line`),
        query: str(value.query, `${context}.query`),
        raw: str(value.raw, `${context}.raw`),
        unsupported: nullableStr(value.unsupported, `${context}.unsupported`),
        generated: count(value.generated, `${context}.generated`),
        problems: Object.freeze(arr(value.problems, `${context}.problems`).map((p, i) => decodeProblem(p, `${context}.problems[${i}]`))) as AutoRuleProblem[],
    });
}

export function decodeAutoRules(raw: unknown): AutoRulesListing {
    const value = obj(raw, "auto rules");
    return Object.freeze({
        inForce: bool(value.inForce, "auto rules.inForce"),
        conf: nullableStr(value.conf, "auto rules.conf"),
        confHijacked: bool(value.confHijacked, "auto rules.confHijacked"),
        rules: Object.freeze(arr(value.rules, "auto rules.rules").map((r, i) => decodeRule(r, `auto rules.rules[${i}]`))) as AutoRule[],
    });
}
