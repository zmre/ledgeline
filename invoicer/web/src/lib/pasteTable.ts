// Spreadsheet paste for the prose fields (invoicer PLAN, Phase 4 feedback
// H12): cells copied from Excel, Numbers or Google Sheets arrive as an HTML
// `<table>` and as tab-separated text. Either becomes a markdown table (the
// first row its header), which the field then reads as a table. No
// dependency: the browser's own DOMParser reads the HTML.

/** A markdown table for `rows` (the first is the header), or null when it isn't a table (fewer than two columns, or no rows). */
export function markdownTable(rows: readonly (readonly string[])[]): string | null {
    const clean = rows.map((row) =>
        row.map((cell) =>
            cell
                .replace(/\r?\n|\r/g, " ")
                .replace(/\s+/g, " ")
                .trim()
        )
    );
    // Trailing blank rows and columns are a spreadsheet's selection edge, not content.
    while (clean.length > 0 && clean[clean.length - 1].every((c) => c === "")) clean.pop();
    const width = clean.reduce((most, row) => {
        let last = row.length;
        while (last > 0 && row[last - 1] === "") last -= 1;
        return Math.max(most, last);
    }, 0);
    if (clean.length === 0 || width < 2) return null;
    const escape = (cell: string): string => cell.replace(/\\/g, "\\\\").replace(/\|/g, "\\|");
    const line = (row: readonly string[]): string => `| ${Array.from({length: width}, (_, i) => escape(row[i] ?? "") || " ").join(" | ")} |`;
    const [header, ...body] = clean;
    return [line(header), `| ${Array.from({length: width}, () => "---").join(" | ")} |`, ...body.map(line)].join("\n");
}

/** Tab-separated text's rows (a quoted cell may hold tabs, newlines and doubled quotes, as Excel writes them). */
export function parseTsv(text: string): string[][] {
    const rows: string[][] = [];
    let row: string[] = [];
    let cell = "";
    let quoted = false;
    const src = text.replace(/\r\n?/g, "\n");
    for (let i = 0; i < src.length; i += 1) {
        const ch = src[i];
        if (quoted) {
            if (ch === '"' && src[i + 1] === '"') {
                cell += '"';
                i += 1;
            } else if (ch === '"') quoted = false;
            else cell += ch;
        } else if (ch === '"' && cell === "") quoted = true;
        else if (ch === "\t") {
            row.push(cell);
            cell = "";
        } else if (ch === "\n") {
            row.push(cell);
            rows.push(row);
            row = [];
            cell = "";
        } else cell += ch;
    }
    if (cell !== "" || row.length > 0) {
        row.push(cell);
        rows.push(row);
    }
    return rows;
}

/** Whether plain text looks like cells from a spreadsheet: every non-blank line has a tab, and there are at least two lines or two tabs. */
export function looksLikeTsv(text: string): boolean {
    const lines = text
        .replace(/\r\n?/g, "\n")
        .split("\n")
        .filter((l) => l.trim() !== "");
    if (lines.length === 0 || !lines.every((l) => l.includes("\t"))) return false;
    return lines.length >= 2 || (lines[0].match(/\t/g)?.length ?? 0) >= 2;
}

/** The first `<table>` in `html` as rows of cell text (a `colspan` repeats nothing: the cell, then blanks), or null. */
export function htmlTableRows(html: string, parse: (html: string) => Document = (h) => new DOMParser().parseFromString(h, "text/html")): string[][] | null {
    if (!/<table[\s>]/i.test(html)) return null;
    const table = parse(html).querySelector("table");
    if (table === null) return null;
    const rows: string[][] = [];
    for (const tr of Array.from(table.querySelectorAll("tr"))) {
        const cells: string[] = [];
        for (const cell of Array.from(tr.querySelectorAll("th, td"))) {
            if (cell.closest("tr") !== tr) continue; // a nested table's cells
            cells.push((cell.textContent ?? "").trim());
            const span = Number(cell.getAttribute("colspan") ?? "1");
            for (let i = 1; i < span && i < 50; i += 1) cells.push("");
        }
        rows.push(cells);
    }
    return rows.length === 0 ? null : rows;
}

/** The markdown table a paste carries (`html` first, else tab-separated `text`), or null for an ordinary paste. */
export function pastedTable(html: string, text: string): string | null {
    const fromHtml = html === "" ? null : htmlTableRows(html);
    if (fromHtml !== null) {
        const md = markdownTable(fromHtml);
        if (md !== null) return md;
    }
    return looksLikeTsv(text) ? markdownTable(parseTsv(text)) : null;
}

/** An empty table to start from (Insert table). */
export const EMPTY_TABLE = "| Column | Column |\n| --- | --- |\n|   |   |";

/** `value` with `block` inserted at `at` (the end when null), set off by blank lines; and where the caret goes after it. */
export function insertBlock(value: string, block: string, at: number | null = null): {value: string; caret: number} {
    const pos = at === null ? value.length : Math.max(0, Math.min(at, value.length));
    const before = value.slice(0, pos).replace(/\s+$/, "");
    const after = value.slice(pos).replace(/^\s+/, "");
    const head = before === "" ? "" : `${before}\n\n`;
    const tail = after === "" ? "\n" : `\n\n${after}`;
    return {value: `${head}${block}${tail}`, caret: head.length + block.length};
}
