// Account tree + name utilities (WP-02). Pure TS: no Svelte/DOM imports.

export interface AccountNode {
    name: string;
    fullName: string;
    children: AccountNode[];
}

/**
 * Build a tree from flat account names (e.g. /accountnames). Intermediate
 * ancestors are created even when absent from the input; siblings are sorted.
 */
export function buildAccountTree(names: string[]): AccountNode[] {
    const roots: AccountNode[] = [];
    const byFullName = new Map<string, AccountNode>();
    for (const fullName of [...names].sort()) {
        if (fullName === "") continue;
        let path = "";
        let siblings = roots;
        for (const segment of fullName.split(":")) {
            path = path === "" ? segment : `${path}:${segment}`;
            let node = byFullName.get(path);
            if (node === undefined) {
                node = {name: segment, fullName: path, children: []};
                byFullName.set(path, node);
                siblings.push(node);
            }
            siblings = node.children;
        }
    }
    return roots;
}

/** Clamp an account name to `depth` segments: ("a:b:c", 2) → "a:b". */
export function clampAccount(name: string, depth: number): string {
    return name.split(":").slice(0, depth).join(":");
}

/** True when `account` is `selected` itself or any of its sub-accounts. */
export function accountMatches(selected: string, account: string): boolean {
    return account === selected || account.startsWith(selected + ":");
}

/**
 * Fallback character budget, used ONLY where nothing has measured the real chip.
 *
 * A character count is not a width. Thirty `l`s and thirty `W`s differ by more
 * than a factor of two in any proportional font, and the chip this has to fit
 * is a fraction of a table column that moves with the window and with which
 * columns the user has enabled — so a fixed count is right at one size and
 * wrong at every other. Tuned to the widest case it under-fills everywhere
 * else, which is exactly the regression this replaced: names were abbreviated
 * to thirty characters inside chips with room for closer to forty, and the
 * slack piled up as dead space at the end of the column.
 *
 * So this is now a last resort rather than the design: it applies to the narrow
 * card layout, to an engine with no 2D canvas to measure with, and to the first
 * frame before the accounts column has reported its size. Everything else goes
 * through `fitAccount` against measured pixels instead.
 *
 * Deliberately left at thirty even though the 45% chip cap it was once tuned
 * against is gone, so on the table it now errs SMALL. That is the direction to
 * err in: too small wastes a little width and keeps the leaf, too large
 * overflows and hands the reader a clipped fragment. A fallback's job is to be
 * unremarkable, not optimal.
 */
export const ACCOUNT_LABEL_BUDGET = 30;

/**
 * How wide `text` renders, in whatever unit the caller is working in — CSS
 * pixels from a canvas for the real thing, visible characters for the fallback.
 * Injecting it is what keeps this module pure and what lets tests state the
 * fill invariant against a deliberately lumpy proportional font.
 */
export type MeasureText = (text: string) => number;

// Ancestor widths tried in order. Three characters is still a word you can read
// past (`exp:`, `ass:`, `liab` → `lia:`); one is the last thing worth saying
// before the segment may as well be gone.
const ANCESTOR_WIDTHS = [3, 1];

// Both the measuring and the cutting below count what a reader SEES, which is
// neither `String.length` (UTF-16 units: `"🇺🇸".length` is 4, and `.slice(1)`
// leaves a lone surrogate that renders as `�`) nor code points (`"🇺🇸"` is two,
// and half a flag is a stray letter in a box). `Intl.Segmenter` has been in
// every engine this app runs in — WKWebView, WebKitGTK, Chromium, node — for
// years, and this module is pure enough to be worth getting exactly right.
const GRAPHEMES = new Intl.Segmenter(undefined, {granularity: "grapheme"});

function cells(text: string): string[] {
    return [...GRAPHEMES.segment(text)].map((cell) => cell.segment);
}

function clip(segment: string, width: number): string {
    const glyphs = cells(segment);
    return glyphs.length <= width ? segment : glyphs.slice(0, width).join("");
}

/**
 * Every way this module is willing to render `name`, WIDEST FIRST, each one
 * strictly narrower than the one before.
 *
 * The ladder spends ANCESTORS and never the leaf. `expenses:auto:maintenance`
 * is twenty-five characters of which the reader wants the last eleven; a plain
 * CSS ellipsis keeps `expenses:auto:ma…`, the half that says nothing. Shortening
 * leading segments instead keeps the part that identifies the account:
 *
 *   expenses:household:repairs:plumbing → exp:household:repairs:plumbing
 *   assets:morganstanley:pw-roth-ira:cash → ass:mor:pw-roth-ira:cash
 *
 * No segment is ever dropped, so every rung is still a path of the same shape
 * and depth as the real one. Ancestors go to three characters left to right
 * (still a word you can read past: `exp:`, `ass:`, `lia:`), then to one, which
 * is the last thing worth saying before a segment may as well be gone.
 *
 * Yielding the ladder rather than walking it internally is what makes "fills
 * the space" checkable: the caller takes the FIRST rung that fits, so the rung
 * before it provably did not, and no width is given up that was available.
 */
export function* accountRenderings(name: string): Generator<string> {
    const segments = name.split(":");
    let last = segments.join(":");
    yield last;
    for (const width of ANCESTOR_WIDTHS) {
        // `length - 1`: the last segment is the leaf and is never a candidate.
        for (let i = 0; i < segments.length - 1; i++) {
            segments[i] = clip(segments[i], width);
            const rendering = segments.join(":");
            // A segment already at or under `width` clips to itself; skipping
            // the repeat keeps every rung genuinely narrower than the last.
            if (rendering === last) continue;
            last = rendering;
            yield rendering;
        }
    }
}

/**
 * The widest rendering of `name` that fits in `room`, measured by `measure`.
 *
 * Returns the full name when it fits — abbreviating something that had space is
 * pure loss, and losing it silently is what left a third of the accounts column
 * empty. When nothing fits, the narrowest rung is returned rather than the name
 * being mangled further here: the label clips it from the LEFT in CSS, which is
 * the same bargain one step on (`…aintenance`), and the leaf survives either
 * way.
 */
export function fitAccount(name: string, room: number, measure: MeasureText): string {
    let narrowest = name;
    for (const rendering of accountRenderings(name)) {
        narrowest = rendering;
        if (measure(rendering) <= room) return rendering;
    }
    return narrowest;
}

/** Shorten an account to `budget` visible characters. The measured `fitAccount` is preferred; see ACCOUNT_LABEL_BUDGET. */
export function abbreviateAccount(name: string, budget: number = ACCOUNT_LABEL_BUDGET): string {
    return fitAccount(name, budget, (text) => cells(text).length);
}

/**
 * Split `room` between items whose natural widths are `naturals`, so that no
 * item is cut while another has more than it needs.
 *
 * This is max-min fairness — fill every item that fits under an equal share,
 * then divide what is left among the rest and repeat. Two accounts on a 300px
 * line wanting 100 and 400 get 100 and 200: the short one is not touched AT ALL
 * and every pixel it did not want goes to the long one.
 *
 * Note what this is NOT. It is not an imitation of flexbox's proportional
 * shrink, which would have cut the short name to 60px to no one's benefit. The
 * caller's job is to choose strings whose TOTAL fits the line; once they do,
 * the line does not overflow, flexbox has nothing to shrink, and every chip
 * renders exactly the string it was fitted for. CSS shrinking is then only the
 * safety net for the case where nothing could be measured.
 *
 * The layout this replaced capped each chip at 45% of the cell independently,
 * so a short source could not lend its slack to a long destination however
 * little of its own 45% it used, and the remainder pooled unusably at the end
 * of the row.
 */
export function shareWidths(naturals: readonly number[], room: number): number[] {
    const shares = naturals.map(() => 0);
    const pending = naturals.map((_, index) => index);
    let left = Math.max(0, room);
    while (pending.length > 0) {
        const fair = left / pending.length;
        // The narrowest outstanding item decides: if even it wants more than an
        // equal share, nobody can be satisfied and everyone takes the share.
        const next = pending.reduce((min, index) => (naturals[index] < naturals[min] ? index : min), pending[0]);
        if (naturals[next] > fair) {
            for (const index of pending) shares[index] = fair;
            break;
        }
        shares[next] = Math.max(0, naturals[next]);
        left -= shares[next];
        pending.splice(pending.indexOf(next), 1);
    }
    return shares;
}

export type RootCategory = "asset" | "liability" | "equity" | "revenue" | "expense" | "other";

/** Categorize by hledger-convention root account name (assets*, liabilities*, equity*, revenues|income*, expenses*). */
export function categorize(account: string): RootCategory {
    const root = account.split(":", 1)[0].toLowerCase();
    if (root.startsWith("asset")) return "asset";
    if (root.startsWith("liabilit")) return "liability";
    if (root.startsWith("equity")) return "equity";
    if (root.startsWith("revenue") || root.startsWith("income")) return "revenue";
    if (root.startsWith("expense")) return "expense";
    return "other";
}
