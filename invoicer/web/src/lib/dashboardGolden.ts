// Tests only: `GET /api/dashboard?today=2026-11-22` on the fixtures, as the
// server's router test records it (crates/ledgeline-invoicer/tests/snapshots/),
// so the SPA's tests draw what the server really answers. Read from the
// working directory (vitest runs in invoicer/web): in the components project
// `import.meta.url` is vite's `/@fs/…` URL, not a file.
import {readFileSync} from "node:fs";
import {resolve} from "node:path";
import type {Dashboard} from "./dashboard.js";

export function golden(name: "accrual" | "cash"): Dashboard {
    const file = resolve(process.cwd(), "../crates/ledgeline-invoicer/tests/snapshots", `${name}-api-dashboard.json`);
    return JSON.parse(readFileSync(file, "utf8")) as Dashboard;
}
