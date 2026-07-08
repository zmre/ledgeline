# WP-09: Fixtures, Golden Tests, E2E

Read `plans/00-overview.md` first. Load the `hledger` skill before authoring journal files.

## Scope

Two phases with different dependencies:

- **Phase A (fixture authoring)** — depends only on repo root: `fixtures/sample.journal`, `scripts/gen-golden.sh`, `scripts/snapshot-api.sh`, committed golden + API-snapshot JSON. Can start day one, parallel with WP-02.
- **Phase B (tests)** — golden vitest tests for the report engine (after WP-06) and normalizer (after WP-02); playwright e2e smoke (after WP-03 + WP-04).

## Phase A: fixtures

### `fixtures/sample.journal`

Hand-authored, ~24 months ending recently, deliberately covering:

- Multi-commodity: USD (`$`), EUR, and a stock (e.g. `AAPL`) bought with cost `@` notation
- `P` price directives for EUR and AAPL at several dates (net-worth valuation)
- Deep accounts (≥4 segments, e.g. `assets:broker:taxable:aapl`), plus `assets:bank:checking`, `liabilities:cc:visa`, `equity:opening`, `income:salary`, `expenses:{food:groceries,housing:rent,...}`
- Account + commodity declarations (`account ... ; type:` tags, `commodity` styles incl. digit groups)
- All three statuses (`*`, `!`, unmarked); transaction + posting comments; tags (`tag:value`)
- A multi-posting split (one txn, 3+ postings); an elided-amount posting; a balance assertion
- Deliberate problem records for WP-08: one posting to `expenses:unknown`, one pending txn, one txn with empty description
- Varied amount styles: digit-grouped `$1,234.56`, comma-decimal EUR style

Must pass `hledger -f fixtures/sample.journal check` (basic; document any strict-mode exceptions).

### `scripts/gen-golden.sh`

Regenerates `fixtures/golden/` from the sample via the real CLI (goldens are committed; regeneration on hledger upgrades is a reviewable diff):

```sh
hledger -f fixtures/sample.journal bs -O json --depth 1 -e 2026-07-01 > fixtures/golden/bs-d1.json
hledger -f fixtures/sample.journal bs -O json --depth 3 -e 2026-07-01 > fixtures/golden/bs-d3.json
hledger -f fixtures/sample.journal is -O json -b 2026-01-01 -e 2026-07-01 --depth 2 > fixtures/golden/is-d2.json
hledger -f fixtures/sample.journal cashflow -O json -M -b 2026-01-01 > fixtures/golden/cf-monthly.json
hledger -f fixtures/sample.journal bal -O json --value=end,$ -e 2026-07-01 assets liabilities > fixtures/golden/networth-spot.json
hledger --version > fixtures/golden/HLEDGER_VERSION
```

(Exact flags may be tuned; keep dates fixed constants, never "today", so goldens are stable.)

### `scripts/snapshot-api.sh`

With `just serve-api` running: curl `/version`, `/transactions`, `/accountnames`, `/prices`, `/commodities` into `fixtures/api/v1.52/*.json` (directory named from `/version`). These are the normalizer's regression net; add `v2.0/` when the preview is packaged.

## Phase B: tests

### Golden vitest tests

- `web/src/lib/api/normalize.test.ts`: normalize `fixtures/api/v1.52/transactions.json`; assert counts, statuses, a known Dec mantissa/places, haystack content, frozen objects. Synthetic 2.0-shaped sample (aprice→acost) asserted equivalent.
- `web/src/lib/reports/golden.test.ts`: run engine (WP-06) on normalized API snapshot; compare against `fixtures/golden/*.json` through a small adapter mapping hledger's report JSON to `{account → {mantissa, places}}` pairs. **Compare exact mantissa/places, never floats.** Vite `?raw`/json imports or fs read via vitest config — fixtures dir is outside `web/`, configure `server.fs.allow` or an alias.

### Playwright e2e smoke (`web/e2e/smoke.test.ts`)

- `playwright.config.ts` `webServer`: launch BOTH `hledger-web -f ../fixtures/sample.journal --serve-api --cors='*' --allow=view --port 5099` and `vite preview` of the built SPA
- Test seeds `localStorage["ledgeline.settings.v1"]` with `http://127.0.0.1:5099`, then asserts: table renders expected row count for "all" preset; "this month" preset filters; selecting an account subtree narrows the totals footer to a known value; reports tab balance sheet shows two known fixture numbers; problems badge shows the deliberate problem count (if WP-08 landed — otherwise skip-marked)
- Nix: uncomment `playwright-driver.browsers` + env exports in `flake.nix` (see comments there)

## Key files created

`fixtures/sample.journal`, `fixtures/golden/*`, `fixtures/api/v1.52/*`, `scripts/{gen-golden.sh,snapshot-api.sh}` (executable, `set -euo pipefail`), `web/src/lib/api/normalize.test.ts`, `web/src/lib/reports/golden.test.ts`, `web/e2e/smoke.test.ts`, playwright webServer config

## Depends on / parallel

Phase A: nothing but repo root — start immediately, parallel with WP-02. Phase B: normalize tests after WP-02; golden tests after WP-06; e2e after WP-03+04 (report assertions after WP-07).

## Definition of done

- `hledger -f fixtures/sample.journal check` passes; `just golden` regenerates deterministically (byte-identical on re-run)
- Golden + normalizer tests green via `just test`; e2e green via `just e2e` in the nix shell
- Goldens and API snapshots committed with `HLEDGER_VERSION` stamp
- Commits: `feat: sample journal and golden fixtures`, `test: golden report tests and e2e smoke`
