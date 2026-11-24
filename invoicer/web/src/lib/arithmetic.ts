// A DISPLAY-ONLY mirror of the bill table's arithmetic (invoicer ARCHITECTURE
// §3.3), so the grid can show amounts while the server's answer is on its way.
// The server (invoicer_core::table) is the only calculator: what this computes
// is shown muted and labelled until `POST /api/invoices/{num}/compute`
// answers, and then replaced. Its tests mirror the Rust goldens, so the two
// rarely disagree; when they do, the server wins.
//
// Money is never a float: every number is a decimal mantissa (BigInt) with a
// scale, rounded half away from zero to cents per row, as the engine does.

/** What a column holds, as the server names its roles. */
export type Role = "description" | "quantity" | "rate" | "amount" | "date" | "unit" | "item" | "books" | "other";

/** How a row reads (the server's `kind`). */
export type RowKind = "item" | "adjustment" | "fixed-adjustment" | "total" | "spacer";

/** A decimal: `mantissa / 10^scale`. */
export interface Dec {
    m: bigint;
    s: number;
}

/** One row, computed. */
export interface LocalRow {
    kind: RowKind;
    /** In cents; null for a spacer, or an item with no amount to show. */
    cents: bigint | null;
    /** For an adjustment: the 0-based row it applies to. */
    appliesTo: number | null;
    /** For a percentage adjustment: the percentage as written, unsigned of `%` ("-10"). */
    percent: string | null;
}

/** A whole table, computed. */
export interface LocalTable {
    rows: LocalRow[];
    /** The amount due: the last total row's, else the sum of every non-total row. */
    total: bigint;
}

/** The total words the engine recognizes by default (`[invoicer.words]` may add more). */
export const DEFAULT_TOTAL_WORDS = ["Subtotal", "Total", "Total due", "Balance due", "Amount due"];

const pow10 = (n: number): bigint => 10n ** BigInt(n);

/** A cell's text without bold markers, `&nbsp;` and padding. */
export function plain(text: string): string {
    let t = text.replace(/&nbsp;/g, " ").trim();
    const bold = /^(\*\*|__)(.*)\1$/.exec(t);
    if (bold !== null) t = bold[2].trim();
    return t;
}

/** Whether a cell is blank (empty or `&nbsp;`). */
export const isBlank = (text: string): boolean => plain(text) === "";

/** Whether a cell is bold (`**…**`). */
export const isBold = (text: string): boolean => /^(\*\*|__).+\1$/.test(text.trim());

/** Read an unsigned decimal ("1,234.5"), or null. Thousands separators are commas (display only). */
function unsigned(text: string): Dec | null {
    const t = text.replace(/[,_' ]/g, "");
    if (!/^\d+(\.\d+)?$|^\.\d+$/.test(t)) return null;
    const [whole, frac = ""] = t.split(".");
    return {m: BigInt((whole === "" ? "0" : whole) + frac), s: frac.length};
}

/** A signed number from a cell: its sign and whether it was explicit. */
function signed(text: string): {sign: 1n | -1n; explicit: boolean; body: string} {
    let t = plain(text).replace(/−/g, "-");
    let sign: 1n | -1n = 1n;
    let explicit = false;
    // "-$800", "- $800", "+$12", "$-800"
    const lead = /^([+-])\s*/.exec(t);
    if (lead !== null) {
        sign = lead[1] === "-" ? -1n : 1n;
        explicit = true;
        t = t.slice(lead[0].length);
    }
    const inner = /^([^\d.+-]*)\s*([+-])\s*/.exec(t);
    if (!explicit && inner !== null && inner[1] !== "") {
        sign = inner[2] === "-" ? -1n : 1n;
        explicit = true;
        t = inner[1] + t.slice(inner[0].length);
    }
    return {sign, explicit, body: t};
}

/** A money cell ("$8,000", "-$800.00", "**$16,700**", "1.5"), or null when it isn't one. */
export function parseMoney(text: string): {value: Dec; explicitSign: boolean} | null {
    const {sign, explicit, body} = signed(text);
    const digits = body.replace(/[^\d.,]/g, "");
    const symbols = body.replace(/[\d.,\s]/g, "");
    if (digits === "" || symbols.length > 4) return null;
    const value = unsigned(digits);
    if (value === null) return null;
    return {value: {m: sign * value.m, s: value.s}, explicitSign: explicit};
}

/** A percentage cell ("-10%", "8.25", "8.25 %"): the number, or null when it isn't one (money has a symbol). */
export function parsePercent(text: string): Dec | null {
    const {sign, body} = signed(text);
    const t = body.replace(/%\s*$/, "").trim();
    const value = unsigned(t);
    return value === null ? null : {m: sign * value.m, s: value.s};
}

/** Round `d` to `scale` places, half away from zero. */
export function round(d: Dec, scale = 2): bigint {
    if (d.s <= scale) return d.m * pow10(scale - d.s);
    const div = pow10(d.s - scale);
    const q = d.m / div;
    const r = d.m % div;
    const twice = (r < 0n ? -r : r) * 2n;
    if (twice >= div) return d.m < 0n ? q - 1n : q + 1n;
    return q;
}

const mul = (a: Dec, b: Dec): Dec => ({m: a.m * b.m, s: a.s + b.s});

/** Cents as money in the table's style, sign before the symbol: `-$1,637.21`. */
export function formatMoney(cents: bigint, commodity = "$"): string {
    const negative = cents < 0n;
    const abs = negative ? -cents : cents;
    const whole = (abs / 100n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    const frac = (abs % 100n).toString().padStart(2, "0");
    const number = `${whole}.${frac}`;
    const prefix = commodity.length <= 1 || !/^[A-Za-z]+$/.test(commodity);
    const body = prefix ? `${commodity}${number}` : `${number} ${commodity}`;
    return negative ? `-${body}` : body;
}

/** Whether `text` is one of the total words. */
function isTotalWord(text: string, words: readonly string[]): boolean {
    const t = plain(text).toLowerCase();
    return words.some((w) => w.toLowerCase() === t);
}

/**
 * Compute a table: `roles` per column, `rows` of raw cells. The rules are the
 * engine's (§3.3): spacer, total, percentage adjustment, fixed adjustment
 * (an explicit sign, no quantity, no rate), item (qty × rate, else as written).
 */
export function computeTable(roles: readonly Role[], rows: readonly (readonly string[])[], totalWords: readonly string[] = DEFAULT_TOTAL_WORDS): LocalTable {
    const col = (role: Role): number => roles.indexOf(role);
    const cell = (row: readonly string[], role: Role): string => {
        const at = col(role);
        return at < 0 ? "" : (row[at] ?? "");
    };
    const out: LocalRow[] = [];
    let sum = 0n;
    let lastTotal: bigint | null = null;
    rows.forEach((row) => {
        if (row.every(isBlank)) {
            out.push({kind: "spacer", cents: null, appliesTo: null, percent: null});
            return;
        }
        const totalCell = row.some((text, at) => roles[at] !== "amount" && roles[at] !== "books" && isBold(text) && isTotalWord(text, totalWords));
        if (totalCell) {
            out.push({kind: "total", cents: sum, appliesTo: null, percent: null});
            lastTotal = sum;
            return;
        }
        const qty = cell(row, "quantity");
        const rate = cell(row, "rate");
        const amount = cell(row, "amount");
        if (isBlank(qty) && !isBlank(rate)) {
            const pct = parsePercent(rate);
            if (pct !== null) {
                let target: number | null = null;
                for (let i = out.length - 1; i >= 0; i -= 1) {
                    if (out[i].kind === "item" || out[i].kind === "total") {
                        target = i;
                        break;
                    }
                }
                const base = target === null ? null : out[target].cents;
                const cents = base === null ? null : round({m: base * pct.m, s: 2 + pct.s + 2}, 2);
                out.push({kind: "adjustment", cents, appliesTo: target, percent: plain(rate).replace(/%\s*$/, "").trim()});
                if (cents !== null) sum += cents;
                return;
            }
        }
        if (isBlank(qty) && isBlank(rate)) {
            const written = parseMoney(amount);
            if (written !== null && written.explicitSign) {
                const cents = round(written.value);
                out.push({kind: "fixed-adjustment", cents, appliesTo: null, percent: null});
                sum += cents;
                return;
            }
        }
        const q = parseMoney(qty);
        const r = parseMoney(rate);
        let cents: bigint | null = null;
        if (q !== null && r !== null) cents = round(mul(q.value, r.value));
        else {
            const written = parseMoney(amount);
            if (written !== null) cents = round(written.value);
        }
        out.push({kind: "item", cents, appliesTo: null, percent: null});
        if (cents !== null) sum += cents;
    });
    return {rows: out, total: lastTotal ?? sum};
}
