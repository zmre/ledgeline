// Shared harness for the docs screenshots: the per-spec engine choice, the
// annotation overlay, and the `shot` helper that writes into docs/.
//
// Annotations are drawn IN the page (plans/25: no ImageMagick), as one
// fixed-position SVG computed from locator bounding boxes. Because the overlay
// is `position: fixed` at the viewport origin, a bounding box (viewport
// coordinates) is also an SVG coordinate, and so is a screenshot `clip`.
import {mkdirSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {expect, test as base, type Locator, type Page} from "@playwright/test";
import {DOCS_ENGINES, DOCS_TOKEN, type DocsJournal} from "../playwright.docs.config";

/** 2026-07-08 local, the clock every e2e spec pins (web/e2e/smoke.e2e.ts). */
export const FIXED_NOW = new Date(2026, 6, 8, 12, 0, 0);

const engineUrl = (journal: DocsJournal) => `http://127.0.0.1:${DOCS_ENGINES[journal]}`;

/** FIXED_NOW as the engine's `YYYY-MM-DD`. */
const FIXED_TODAY = "2026-07-08";

/**
 * `page.clock` pins the BROWSER's today, but Projections leaves its date to the
 * engine, which falls back to its own wall clock: the seed's averaging window
 * (`end`) and the run's opening-balance date (`asOf`). Unpinned, every month
 * label in the projection shots moves with the day the harness runs. So the
 * two requests get the pinned date filled in when the SPA leaves it out.
 */
const pinEngineToday = async (page: Page): Promise<void> => {
    await page.route("**/api/projections/seed*", async (route) => {
        const url = new URL(route.request().url());
        if (url.searchParams.has("end")) return route.continue();
        url.searchParams.set("end", FIXED_TODAY);
        return route.continue({url: url.toString()});
    });
    await page.route("**/api/projections/run", async (route) => {
        const body = route.request().postDataJSON() as Record<string, unknown> | null;
        if (route.request().method() !== "POST" || body === null || "asOf" in body) return route.continue();
        return route.continue({postData: JSON.stringify({...body, asOf: FIXED_TODAY})});
    });
};

type DocsOptions = {
    /** Which staged fixture set's engine the page talks to. */
    journal: DocsJournal;
    /** Extra `ledgeline.settings.v1` keys (e.g. `insightsOpen: false`). */
    settings: Readonly<Record<string, unknown>>;
};

export const test = base.extend<DocsOptions>({
    journal: ["sample", {option: true}],
    settings: [{}, {option: true}],
    // Same-origin: the page is served by the very engine it talks to.
    baseURL: async ({journal}, use) => use(engineUrl(journal)),
    page: async ({page, journal, settings}, use) => {
        await page.clock.setFixedTime(FIXED_NOW);
        // ONE key holding everything: seeding serverUrl and the token separately
        // leaves the app with no server and the first-run modal over the page.
        const seeded = {...settings, serverUrl: engineUrl(journal), serverToken: DOCS_TOKEN};
        await page.addInitScript((value) => localStorage.setItem("ledgeline.settings.v1", value), JSON.stringify(seeded));
        await pinEngineToday(page);
        await use(page);
    },
});
export {expect};

// ---------------------------------------------------------------------------
// Geometry

export type Rect = Readonly<{x: number; y: number; width: number; height: number}>;

const union = (rects: ReadonlyArray<Rect>): Rect => {
    const x0 = Math.min(...rects.map((r) => r.x));
    const y0 = Math.min(...rects.map((r) => r.y));
    const x1 = Math.max(...rects.map((r) => r.x + r.width));
    const y1 = Math.max(...rects.map((r) => r.y + r.height));
    return {x: x0, y: y0, width: x1 - x0, height: y1 - y0};
};

const inflate = (r: Rect, by: number): Rect => ({x: r.x - by, y: r.y - by, width: r.width + 2 * by, height: r.height + 2 * by});

const boxOf = async (target: Locator | Rect): Promise<Rect> => {
    if (!("boundingBox" in target)) return target;
    await target.scrollIntoViewIfNeeded();
    const box = await target.boundingBox();
    if (!box) throw new Error(`annotation target has no box (hidden or detached): ${target}`);
    return box;
};

/** One rect covering several targets, for a single box or circle around a group of controls. */
export const around = async (targets: ReadonlyArray<Locator | Rect>): Promise<Rect> => {
    const rects: Rect[] = [];
    for (const t of targets) rects.push(await boxOf(t));
    return union(rects);
};

// ---------------------------------------------------------------------------
// Annotations

/** Bold enough to read at 50% zoom on both the dark UI and a light page. */
export const ANNOTATION_COLOR = "#e8590c";
const STROKE = 3.5;
const OVERLAY_ID = "docs-annotations";

export type Annotation = Readonly<
    | {kind: "circle"; target: Locator | Rect; pad?: number}
    | {kind: "box"; target: Locator | Rect; pad?: number}
    | {
          kind: "arrow";
          target: Locator | Rect;
          /** Where the tail sits, relative to the target's centre. Default: up-left. */
          from?: Readonly<{dx: number; dy: number}>;
          label?: string;
      }
>;

/** The point where the segment from `p` towards the centre of `r` crosses `r`'s edge. */
const edgePoint = (p: {x: number; y: number}, r: Rect): {x: number; y: number} => {
    const cx = r.x + r.width / 2;
    const cy = r.y + r.height / 2;
    const dx = p.x - cx;
    const dy = p.y - cy;
    const sx = dx === 0 ? Infinity : r.width / 2 / Math.abs(dx);
    const sy = dy === 0 ? Infinity : r.height / 2 / Math.abs(dy);
    const s = Math.min(sx, sy);
    return {x: cx + dx * s, y: cy + dy * s};
};

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

type Drawn = Readonly<{svg: string; bounds: Rect}>;

const draw = (a: Annotation, r: Rect): Drawn => {
    switch (a.kind) {
        case "circle": {
            // Loosely around the padded box. A true circumscribing ellipse (radii ×√2)
            // balloons around wide controls and swallows their neighbours, so the
            // corners may clip slightly: it reads as "this one", which is the point.
            const p = inflate(r, a.pad ?? 4);
            const rx = p.width / 2 + Math.min(18, Math.max(6, p.width * 0.12));
            const ry = p.height / 2 + Math.min(12, Math.max(6, p.height * 0.2));
            const cx = p.x + p.width / 2;
            const cy = p.y + p.height / 2;
            return {
                svg: `<ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="none" stroke="${ANNOTATION_COLOR}" stroke-width="${STROKE}"/>`,
                bounds: inflate({x: cx - rx, y: cy - ry, width: 2 * rx, height: 2 * ry}, STROKE),
            };
        }
        case "box": {
            const p = inflate(r, a.pad ?? 6);
            return {
                svg: `<rect x="${p.x}" y="${p.y}" width="${p.width}" height="${p.height}" rx="8" fill="none" stroke="${ANNOTATION_COLOR}" stroke-width="${STROKE}"/>`,
                bounds: inflate(p, STROKE),
            };
        }
        case "arrow": {
            const {dx, dy} = a.from ?? {dx: -110, dy: -70};
            const cx = r.x + r.width / 2;
            const cy = r.y + r.height / 2;
            const tail = {x: cx + dx, y: cy + dy};
            const head = edgePoint(tail, inflate(r, 6));
            const parts = [
                `<line x1="${tail.x}" y1="${tail.y}" x2="${head.x}" y2="${head.y}" stroke="${ANNOTATION_COLOR}" stroke-width="${STROKE}" stroke-linecap="round" marker-end="url(#docs-arrowhead)"/>`,
            ];
            const rects: Rect[] = [
                inflate({x: Math.min(tail.x, head.x), y: Math.min(tail.y, head.y), width: Math.abs(head.x - tail.x), height: Math.abs(head.y - tail.y)}, 14),
            ];
            if (a.label) {
                // A pill centred on the tail; the line starts underneath it.
                const w = a.label.length * 7.6 + 22;
                const h = 26;
                const pill = {x: tail.x - w / 2, y: tail.y - h / 2, width: w, height: h};
                parts.push(
                    `<rect x="${pill.x}" y="${pill.y}" width="${w}" height="${h}" rx="${h / 2}" fill="${ANNOTATION_COLOR}"/>`,
                    `<text x="${tail.x}" y="${tail.y + 4.5}" text-anchor="middle" fill="#fff" font-family="system-ui, -apple-system, sans-serif" font-size="13" font-weight="700">${esc(a.label)}</text>`
                );
                rects.push(inflate(pill, 2));
            }
            return {svg: parts.join(""), bounds: union(rects)};
        }
    }
};

/**
 * Draws `specs` over the page and returns the union of what was drawn, so a
 * `shot` clip can be widened to keep every mark in frame.
 */
export const annotate = async (page: Page, specs: ReadonlyArray<Annotation>): Promise<Rect> => {
    const drawn: Drawn[] = [];
    for (const a of specs) drawn.push(draw(a, await boxOf(a.target)));
    const markup = [
        `<defs><marker id="docs-arrowhead" viewBox="0 0 10 10" refX="7" refY="5" markerWidth="4" markerHeight="4" orient="auto-start-reverse">`,
        `<path d="M0,0 L10,5 L0,10 z" fill="${ANNOTATION_COLOR}"/></marker>`,
        // A soft dark halo so the strokes hold their edge on light and busy backgrounds.
        `<filter id="docs-halo" x="-10%" y="-10%" width="120%" height="120%"><feDropShadow dx="0" dy="0" stdDeviation="1.2" flood-color="#000" flood-opacity="0.55"/></filter></defs>`,
        `<g filter="url(#docs-halo)">${drawn.map((d) => d.svg).join("")}</g>`,
    ].join("");
    await page.evaluate(
        ([id, inner]) => {
            document.getElementById(id)?.remove();
            const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
            svg.id = id;
            svg.setAttribute("style", "position:fixed;inset:0;width:100vw;height:100vh;pointer-events:none;z-index:2147483647;overflow:visible");
            svg.innerHTML = inner;
            document.body.appendChild(svg);
        },
        [OVERLAY_ID, markup] as const
    );
    return union(drawn.map((d) => d.bounds));
};

/** Removes the overlay `annotate` drew. */
export const clear = async (page: Page): Promise<void> => {
    await page.evaluate((id) => document.getElementById(id)?.remove(), OVERLAY_ID);
};

// ---------------------------------------------------------------------------
// Shots

const DOCS_DIR = resolve(import.meta.dirname, "../../docs");

/**
 * Docs path for a shot. `section` is a folder of the manual (journal, imports,
 * reports, holdings, projections, settings) and the image lands in
 * `docs/<section>/images/<name>.png`. Budget is a single top-level page, so its
 * images go to `docs/images/budget-<name>.png`.
 */
export const shotPath = (section: string, name: string): string =>
    section === "budget" ? resolve(DOCS_DIR, "images", `budget-${name}.png`) : resolve(DOCS_DIR, section, "images", `${name}.png`);

export type ShotOptions = Readonly<{
    /** Region(s) to keep; the union is taken. Omit for the whole viewport. */
    clip?: Locator | Rect | ReadonlyArray<Locator | Rect>;
    /** Margin around `clip`, in CSS px. */
    padding?: number;
}>;

/**
 * Waits for the page to be quiet (no spinners, fonts loaded), then writes a PNG
 * with animations frozen and the caret hidden. Excerpts are clamped to the
 * viewport so a padded clip near an edge cannot fail the screenshot.
 */
export const shot = async (page: Page, section: string, name: string, opts: ShotOptions = {}): Promise<string> => {
    await settle(page);
    const path = shotPath(section, name);
    mkdirSync(dirname(path), {recursive: true});
    const vp = page.viewportSize() ?? {width: 1280, height: 800};
    let clip: Rect | undefined;
    if (opts.clip) {
        const targets = Array.isArray(opts.clip) ? opts.clip : [opts.clip as Locator | Rect];
        const rects: Rect[] = [];
        for (const t of targets) rects.push("boundingBox" in t ? await boxOf(t) : t);
        const r = inflate(union(rects), opts.padding ?? 12);
        const x = Math.max(0, Math.floor(r.x));
        const y = Math.max(0, Math.floor(r.y));
        clip = {x, y, width: Math.min(vp.width, Math.ceil(r.x + r.width)) - x, height: Math.min(vp.height, Math.ceil(r.y + r.height)) - y};
    }
    await page.screenshot({path, clip, animations: "disabled", caret: "hide", scale: "device"});
    return path;
};

/** No loading spinners, no skeletons, fonts ready, two frames painted. */
export const settle = async (page: Page): Promise<void> => {
    await expect(page.locator(".loading, .skeleton, [aria-busy='true']").filter({visible: true})).toHaveCount(0, {timeout: 15_000});
    await page.evaluate(async () => {
        await document.fonts.ready;
        await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    });
};
