// Decoder for GET /api/holdings/benchmarks (WireBenchmarks in
// crates/ledgeline-server/src/benchmarks_api.rs). Its own module rather than a
// section of nativeDecode.ts: the overlay is an optional enhancement with its
// own request, and nothing else decodes through it.
//
// Same stance as nativeDecode: a shape we cannot trust is an ApiShapeError,
// never a quietly-defaulted line — a benchmark drawn from a zero-filled
// misread would be a confident wrong comparison.

import type {BenchmarkLine, BenchmarksResponse} from "$lib/holdings/benchmarks";
import {ApiShapeError} from "./client";

interface RawPoint {
    date?: unknown;
    value?: unknown;
}

interface RawLine {
    symbol?: unknown;
    label?: unknown;
    points?: unknown;
    pricedThrough?: unknown;
    stale?: unknown;
    unvaluedFlows?: unknown;
    error?: unknown;
}

function decodeLine(raw: RawLine | undefined, context: string): BenchmarkLine {
    if (raw === undefined || raw === null || typeof raw.symbol !== "string" || typeof raw.label !== "string" || !Array.isArray(raw.points)) {
        throw new ApiShapeError(`${context}: expected symbol/label/points`);
    }
    const points = (raw.points as RawPoint[]).map((point, i) => {
        if (typeof point?.date !== "string") throw new ApiShapeError(`${context} point #${i}: missing date`);
        const value = point.value;
        if (value !== null && (typeof value !== "number" || !Number.isFinite(value))) {
            throw new ApiShapeError(`${context} point #${i}: value is neither a number nor null`);
        }
        return Object.freeze({date: point.date, value});
    });
    return Object.freeze({
        symbol: raw.symbol,
        label: raw.label,
        points: Object.freeze(points),
        pricedThrough: typeof raw.pricedThrough === "string" ? raw.pricedThrough : null,
        stale: raw.stale === true,
        unvaluedFlows: typeof raw.unvaluedFlows === "number" ? raw.unvaluedFlows : 0,
        error: typeof raw.error === "string" ? raw.error : null,
    });
}

export function decodeBenchmarks(raw: unknown): BenchmarksResponse {
    const body = raw as {base?: unknown; benchmarks?: unknown} | null;
    if (typeof body !== "object" || body === null || typeof body.base !== "string" || !Array.isArray(body.benchmarks)) {
        throw new ApiShapeError("benchmarks: expected base/benchmarks");
    }
    return Object.freeze({
        base: body.base,
        benchmarks: Object.freeze((body.benchmarks as RawLine[]).map((line, i) => decodeLine(line, `benchmark #${i}`))),
    });
}
