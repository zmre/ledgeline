# Plan 43: implementation contract

The plan is `plans/43-bank-sync.md`. This file pins the seams between the
parallel work packages, so each can be built and tested alone and merged
without guesswork. Where this file and the plan disagree, this file wins. Any
deviation gets a note under "Deviations" at the bottom, written by whoever
made it.

## Work packages

| WP  | Owner files                                                                                                   | Depends on      |
| --- | ------------------------------------------------------------------------------------------------------------- | --------------- |
| A   | `crates/ledgeline-core/src/feeds/**` (new), `reimport.rs` (multi-id only), `crates/ledgeline-core/tests/feeds*.rs`, `fixtures/feeds/**` | —               |
| R   | `crates/ledgeline-core/src/rules/share.rs` (new), `rules/generate.rs` (feed template), `rules/matching.rs` (skip field-less files), `rules.rs` (only if share needs a helper), `fixtures/import/shared-categories/**`, `crates/ledgeline-core/tests/rules_share.rs` | —               |
| W   | `web/src/lib/sync/**` (new), `web/src/routes/imports/+page.svelte`, imports tab component, settings tab wiring + `SyncSettingsPanel.svelte`, header dot in `+layout.svelte`, `web/src/lib/api/native.ts` (sync section only), `uiPrefs.ts`/`settings.svelte.ts` (sync keys only) | the wire shapes here |
| S   | `crates/ledgeline-server/src/sync/**` (new), `ui_prefs.rs` (4 keys), `lib.rs` (routes, AppState fields, spawn), `main.rs`/`gui.rs` (spawn, `sync` CLI), `Cargo.toml` (`keyring`), server tests | A, R            |
| D   | `docs/imports/bank-sync.md` (+ index links), screenshots                                                       | all             |

**Shared rules:**

- Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` for the
  crates you touch.
- Use `bun run check` and `bun run test:unit` in `web/`.
- **Never run Playwright** (the e2e ports are shared across worktrees).
- **Never touch the network in tests.** **Never read any journal outside
  `fixtures/`.**
- Don't edit files owned by another WP. If you need something from one, put a
  `MERGE SEAM:` comment where you'd call it and note it below.

---

## WP-A: `ledgeline_core::feeds` (pure, no I/O)

```rust
// feeds/mod.rs
pub mod simplefin;   // token + access URL + /accounts parsing
pub mod normalize;   // FeedAccount -> CSV text the sync rules file reads
pub mod dupes;       // id-known / possible-duplicate split
pub mod config;      // [sync] tables in ledgeline.toml (parse + toml_edit writers)

/// Provider-neutral result of one fetch.
pub struct FeedBatch {
    pub connections: Vec<FeedConnection>,
    pub accounts: Vec<FeedAccount>,
    pub problems: Vec<FeedProblem>,
    pub notices: Vec<String>,          // x-api-message etc., sanitized
}
pub struct FeedConnection { pub id: String, pub name: String, pub org_url: Option<String> }
pub struct FeedAccount {
    pub id: String,                    // provider account id
    pub name: String,
    pub conn_id: Option<String>,
    pub org_name: Option<String>,      // v2 connection name / v1 org.name
    pub org_url: Option<String>,       // org domain/url, for "Fix at SimpleFIN"
    pub currency: FeedCurrency,        // Code("USD") | Url(String)
    pub balance: String,               // decimal text exactly as sent
    pub available_balance: Option<String>,
    pub balance_date: Option<i64>,     // epoch seconds
    pub transactions: Vec<FeedTxn>,
}
pub struct FeedTxn {
    pub id: String,
    pub posted: i64,                   // 0 = not posted
    pub amount: String,                // decimal text, + = money in
    pub description: String,
    pub payee: Option<String>,
    pub memo: Option<String>,
    pub transacted_at: Option<i64>,
    pub pending: bool,
}
pub enum ProblemKind { Gen, GenApi, GenAuth, ConAuth, Act, ActFailed, ActMissingData, NeedsAttention /* v1 text */, Quota, Other }
pub struct FeedProblem {
    pub kind: ProblemKind,
    pub code: String,                  // raw code ("con.auth") or "" for v1 strings
    pub message: String,               // sanitized: control chars stripped, ≤ 500 chars
    pub conn_id: Option<String>,
    pub account_id: Option<String>,
}
```

### `simplefin`

- `decode_setup_token(input: &str) -> Result<SetupToken, TokenError>`
  - Trims whitespace and accepts standard or URL-safe base64, with or without
    padding.
  - The decoded value must be an `https://` URL whose path contains
    `/claim/`.
  - `SetupToken { claim_url: String, host: String }`.
  - `TokenError` variants: `NotBase64`, `NotUrl`, `NotHttps`, `NotAClaimUrl`.
  - **Test-only escape:** `http://127.0.0.1` / `http://localhost` are allowed
    when the `allow_loopback_http: bool` argument is true. Expose it as
    `decode_setup_token_with(input, allow_loopback_http)`; the plain function
    passes false.
- `AccessUrl::parse(text: &str, allow_loopback_http: bool) -> Result<AccessUrl, TokenError>`
  - Fields: `base: String` (scheme://host[:port]/path, with no userinfo and no
    trailing slash), `username`, `password`, `host`.
  - Its `Debug` and `Display` **must redact** the password and username.
  - `to_secret_string()` gives back the full URL, for the keychain.
- `parse_accounts(body: &[u8]) -> Result<FeedBatch, ParseError>`
  - Accepts both v1 (`errors: [string]`, `org` object) and v2 (`errlist`,
    `connections`).
  - Unknown fields are ignored; optional fields are tolerated.
  - Amounts and balances stay text. They must parse as decimals (the existing
    `Dec`/`rust_decimal` the crate uses), or the transaction is dropped and
    reported as an `Other` problem.
  - The `x-api-message` array becomes `notices`.
  - v1 strings containing "may need attention" become `NeedsAttention`.
  - The quota warnings the Bridge sends in `errors` (text mentioning "limit"
    or "quota") become `Quota`.
- Fixtures go in `fixtures/feeds/simplefin/`:
  - `demo-v2.json` and `demo-v1.json`, shaped like the live demo (3 accounts,
    one empty, `payee`/`memo`/`mcc`/`holdings` present);
  - `errors-v2.json` (`con.auth` with `conn_id`, `act.failed` with
    `account_id`, `gen.api` 90-day cap);
  - `pending.json`;
  - `custom-currency.json`;
  - `odd-amounts.json` (`"5"`, `"-0.1"`, `"1,000.00"` → reported).

  All synthetic; write them by hand.

### `normalize`

```rust
pub struct FeedColumns {              // CSV header names; defaults shown
    pub date: String,        // "date"
    pub amount: String,      // "amount"
    pub payee: String,       // "payee"
    pub description: String, // "description"
    pub memo: String,        // "memo"
    pub id: String,          // "id"
    pub transacted: String,  // "transacted"
}
impl Default for FeedColumns { … }

pub struct NormalizeOptions<'a> {
    pub start_date: &'a str,          // ISO; rows dated before are dropped
    pub account_short: &'a str,       // id prefix, see account_short()
    pub columns: &'a FeedColumns,
    pub utc_offset_seconds: i32,      // local offset, for non-midnight epochs
}
pub struct NormRow { pub date: String, pub amount: String, pub payee: String,
                     pub description: String, pub memo: String, pub id: String,
                     pub transacted: String /* "" when same day as date */ }
pub struct Normalized {
    pub rows: Vec<NormRow>,           // sorted by (date, id)
    pub dropped_pending: usize,
    pub dropped_before_start: usize,
    pub dropped_zero: usize,
}
pub fn normalize(account: &FeedAccount, opts: &NormalizeOptions) -> Normalized;
pub fn to_csv(rows: &[NormRow], columns: &FeedColumns) -> String;   // header + RFC4180 quoting
pub fn account_short(connection_id: &str, remote_account_id: &str) -> String; // 10 lowercase hex, STABLE forever (FNV-1a 64 or sha2 if already a dep; never DefaultHasher)
pub fn epoch_to_date(epoch: i64, utc_offset_seconds: i32) -> String;
```

- **Pending:** a transaction with `pending == true` or `posted == 0` is
  always dropped.
- **Dates:** an epoch that's exactly midnight UTC → that UTC date. Anything
  else → the date at `utc_offset_seconds`.
  - `transacted` is filled only when its date differs from `date`.
- **Ids:** `id` = `"{account_short}-{tag_safe(txn.id)}"`. `tag_safe` works
  like `convert::ofx::tag_safe`: it removes commas and collapses whitespace.
  Reuse it if it's public, or make it `pub(crate)`, since it's in core.
- `payee` falls back to `description`.
- Zero amounts are dropped.

### `dupes`

```rust
pub fn known_ids(journal: &Journal) -> HashSet<String>;   // every `id:` tag value on every txn
pub struct Existing { pub index: usize, pub date: String, pub description: String, pub amount: String }
pub struct Held { pub row: NormRow, pub existing: Existing }
pub struct Split { pub already: Vec<String> /*ids*/, pub fresh: Vec<NormRow>, pub held: Vec<Held> }
pub fn split(rows: Vec<NormRow>, journal: &Journal, journal_account: &str,
             known: &HashSet<String>, fetched_ids: &HashSet<String>) -> Split;
```

- A row whose `id` is in `known` goes to `already`. That's silent: a hand
  edit beats a re-download, and no conflict is reported for sync.
- Otherwise, a row is **held** when an existing transaction meets all of:
  - it has a posting to exactly `journal_account` with the same amount
    (decimal equality, the same commodity or none);
  - its date is within ±3 days;
  - it carries no `id` that appears in `fetched_ids` (the prefixed ids of
    every row in this fetch, before any filtering);
  - its normalized payee shares a token of ≥ 3 letters with the row's payee
    or description.

  Normalizing means: lowercase, letters only, split on everything else, and
  drop tokens of digits.
- **Held rows never write.** Each existing transaction can claim at most one
  held row.

### `reimport.rs` change

`build_index` indexes **every** `id` tag value on a transaction, not only
the first. Tests must show that a transaction with two `id:` tags is found
by both. `classify` is unchanged.

### `config`

This module covers the `[sync]` tables in `ledgeline.toml`.

```toml
[[sync.connection]]
id = "4f6c…"              # uuid v4 text; keychain key = "ledgeline.sync.<id>"
name = "SimpleFIN Bridge"
kind = "simplefin"

[[sync.account]]
connection = "4f6c…"
remoteId = "ACT-…"
label = "First Tech · Checking ••1234"
journalAccount = "assets:bank:firsttech:checking"
rules = "imports/rules/firsttech-checking.sync.csv.rules"   # journal-relative
target = "2026.journal"                                     # journal-relative
startDate = 2026-11-01                                      # TOML date
balanceSign = 1                                             # 1 | -1
enabled = true

[[sync.manual]]                                             # optional, per manual account
account = "assets:invest:vanguard"
downloadUrl = "https://…"
nudge = true
```

```rust
pub struct SyncConfig { pub connections: Vec<ConnectionLink>, pub accounts: Vec<AccountLink>, pub manual: Vec<ManualAccount> }
pub fn parse(text: &str) -> (SyncConfig, Vec<String> /*warnings, entry skipped*/);
pub fn upsert_connection(text: &str, c: &ConnectionLink) -> Result<String, ConfigError>;
pub fn upsert_account(text: &str, a: &AccountLink) -> Result<String, ConfigError>;   // keyed by (connection, remoteId)
pub fn remove_account(text: &str, connection: &str, remote_id: &str) -> Result<String, ConfigError>;
pub fn remove_connection(text: &str, id: &str) -> Result<String, ConfigError>;       // keeps its accounts
pub fn upsert_manual(text: &str, m: &ManualAccount) -> Result<String, ConfigError>;
```

- All writers use `toml_edit` and preserve comments, order and every other
  table, `[ui]` included.
- `parse` never fails as a whole: a malformed entry becomes a warning and
  is skipped.
- `parse` checks nothing on the filesystem. The server validates paths.
- `AccountLink::key()` = `account_short(connection, remoteId)`.

---

## WP-R: rules sharing (`rules/share.rs`) and the feed template

```rust
// rules/generate.rs
pub struct FeedRulesParams<'a> {
    pub label: &'a str,                 // "First Tech · Checking ••1234"
    pub provider: &'a str,              // "SimpleFIN"
    pub columns: &'a FeedColumns,       // from feeds::normalize (WP-A). MERGE SEAM if A not merged: define a local mirror struct with the same fields and note it
    pub journal_account: &'a str,
    pub fallback_account: &'a str,      // "expenses:unknown"
    pub categories_include: Option<&'a str>, // sibling file name, already validated by the caller
}
pub fn feed_rules(params: &FeedRulesParams) -> String;
```

The template is the one in plan §3.2:

- `skip 1`
- the `fields` line in `FeedColumns` order;
- `date-format %Y-%m-%d`
- `account1`, `account2`
- `description %<payee col>`
- `date2 %<transacted col>`
- `comment id:%<id col>`
- the optional `include` **last**.

It must pass `just rules-check`; add a fixture copy under the rules-check
glob.

```rust
// rules/share.rs
pub struct SharePlan {
    pub manual_new_text: String,        // manual file without the moved if-blocks, + "include <categories>" as the LAST line
    pub categories_text: String,        // header comment + moved if-blocks, verbatim bytes, original order
    pub moved: usize,
    pub kept: Vec<KeptBlock>,           // { header: String /*first line*/, reason: KeepReason }
    pub columns: FeedColumns,           // sync columns with names BORROWED from the manual file
}
pub enum KeepReason { FieldNotInSync(String), LayoutDependentPattern, Opaque }
pub fn plan_share(manual_text: &str, categories_file_name: &str) -> Result<SharePlan, ShareError>;
pub fn append_include(rules_text: &str, file_name: &str) -> String; // used for a sync file created without one
```

**Which blocks move:**

- **Field matchers:** an `if` block moves when every matcher it uses names a
  field in the shareable set. That set is the manual file's fields that map
  to a sync column (date, amount, payee/description, memo), plus
  whole-record matchers.
- **Whole-record patterns:** these move unless the pattern contains `^`, `$`
  or `,`, which gives `LayoutDependentPattern`.
- **Anything else stays:**
  - `if` tables;
  - blocks `rules.rs` classifies as opaque (`Opaque`);
  - blocks matching on `trntype`, `checknum` and the like
    (`FieldNotInSync`).

**Borrowing names:**

- The column the manual file feeds into `description` (through a
  `description %X` assignment, or a field literally named `description`)
  becomes the sync **payee** column name, `X`.
- When the manual file also has a field literally called `description` that
  isn't the payee source, the sync description column is named `description`.
  Otherwise it's named `sfdescription`, so a collision is impossible.
- `memo`, `amount` and `date` are borrowed the same way when the manual file
  names them. Otherwise the defaults apply.
- Ids always stay `id`, and transacted stays `transacted`, unless those
  collide. On a collision, prefix `sf`.

**What `plan_share` leaves to the server:** byte-identity of `hledger print`
is checked by WP-S, which runs hledger. `plan_share` must be pure and
**round-trip safe**: every byte of the manual file not moved stays put.
Concatenating `manual_new_text` minus its include line with the moved blocks
in their original positions must reproduce the original. Property-test
that.

**`matching.rs`:** a rules file with no `fields` line scores as
"doesn't fit" and is never offered as a candidate.

**`rules.rs`:** an `include` stays keep-only for the editor. `share.rs`
produces the new text itself, as plain text edits. The server writes it only
after validating the sibling name with `discovery::resolve_new`.

---

## WP-S: server (`crates/ledgeline-server/src/sync/`)

### Modules

- `mod.rs`: routes.
- `client.rs`: the `FeedProvider` trait and the reqwest SimpleFIN client.
- `secrets.rs`: the `SecretStore` trait, plus `KeyringStore` and
  `MemoryStore`.
- `state.rs`: `sync-state.json` in `AppDirs.cache_dir`.
- `engine.rs`: one run (fetch → normalize → split → stage CSV → dry run →
  commit).
- `auto.rs`: the pure `plan_sync` and `run_forever`, copying the shape of
  `auto_prices.rs`.
- `link.rs`: the wizard's preview and commit, plus the share preview.
- `wire.rs`: the wire types below.

### Engine rules

- **Lock order:** fetch outside every lock, then take `import_writes`, then
  stage and commit through `import_api`'s own `run_dry_run`/`run_commit`,
  exactly as `run_cli_import` does. Don't re-implement any of the import.
- **The CSV** is written to `<syncSourceDir>/<connection-slug>/<account-slug>/<YYYY-MM-DDTHHMM>.csv`.
  - After a successful commit, delete the `.latest.*` file hledger created
    beside that CSV. Ids and the start date dedup; `.latest` would hide
    late-arriving rows.
  - Prune files older than `syncKeepSourceDays` (0 = never) in that account
    folder only.
- **Held rows** are stored in `sync-state.json` under the account key, each
  with a stable `heldId` = the row id.
  - **Import anyway** stages a one-row CSV through the same path.
  - **It's a duplicate** adds `id:<row id>` as a second tag on the existing
    transaction through the existing transaction editor (an `edit_api` tag
    op).
- **Window:**
  - `start = max(startDate, lastSuccess − 5 days)`;
  - `end = tomorrow` (local);
  - chunks of ≤ 90 days, oldest first;
  - never send `pending=1`;
  - always send `version=2`;
  - repeat `account=` when the run is for one account.
- **HTTP:**
  - User-Agent `Ledgeline/<version> (+https://github.com/zmre/ledgeline)`.
  - Basic auth comes from the parsed `AccessUrl`. Never put credentials in
    the URL.
  - Redirects: none; treat 3xx as an error.
  - Timeout 60 s.
  - HTTPS only, except loopback when the state was built with
    `with_sync_loopback_http()` (tests only).
- **Claim:**
  - `POST` the claim URL with an empty body.
  - 200 → the body is the access URL.
  - 403 with a `cf-ray` header and an HTML body → `edge-blocked`; any other
    403 → `token-used`.
- **Quota** is counted per connection per UTC day in `sync-state.json`:
  - auto runs stop at 6;
  - manual runs warn at 12 and are refused at 16;
  - a `Quota` problem → no more auto runs until the next UTC day.
- **Backoff** on network errors and 5xx: 15 min, doubling, capped at 6 h. It's
  kept in memory.
- **Due rule:** auto is due when `lastSuccess` < today 07:00 local, or when
  `now − lastSuccess ≥ autoSyncEveryHours`.
  - First check at 45 s plus a random 0–600 s jitter, then a 5-min
    wall-clock wake.
  - `wake()` is called when sync prefs change.
- **Not spawned on iOS** (FFI) or under `LEDGELINE_UI_PREFS_IN_BROWSER`.
  Tests use `run_sync_once(now)`.
- **`AppState` builders:**
  - `with_feed_provider(Arc<dyn FeedProvider>)`
  - `with_secret_store(Arc<dyn SecretStore>)`
  - `with_sync_loopback_http()`
  - The default is the real client plus `KeyringStore`.
- **Env `LEDGELINE_FEED_FAKE=<dir>`** swaps in a provider that serves
  `<dir>/accounts.json`. Claim accepts any token whose decoded URL host is
  `fake.simplefin.test`. The secret store is in-memory. This is for e2e and
  docs screenshots, and is compiled only with `--features test-fakes`.
  `just e2e` builds with it.
- **ui_prefs keys,** all `PINNABLE`:
  - `autoSync` (flag, default false);
  - `autoSyncEveryHours` (whole 3..=24, default 6);
  - `syncSourceDir`: a new kind `RelDir`, a relative path with no `..` and
    no leading `/`, default `imports/sync`;
  - `syncKeepSourceDays` (whole 0..=3650, default 90).
- **`ledgeline.toml` writes** go through the same writer and lock as
  `ui_prefs::pin`. Linking may create the file (the documented exception).
- **Paths** (`rules`, `target`, `syncSourceDir`) must resolve inside the
  journal dir via the existing resolvers. Otherwise the link reports a
  `setup` problem and is skipped.
- **CLI** (`ledgeline sync …`):
  - `connect <token> [--name]`
  - `accounts <connection>`
  - `link <connection> <remoteId> --account A --rules R --target T --start D`
  - `status`
  - `now [--connection C]`

  Same engine. This is for dogfooding.

### Routes

All routes need the bearer token. Writes are 403 when `!editing_enabled()`.

| Method | Path                                         | Body → Response                                                                  |
| ------ | -------------------------------------------- | -------------------------------------------------------------------------------- |
| GET    | `/api/sync/status`                           | → `WireSyncStatus`                                                               |
| POST   | `/api/sync/connect`                          | `{setupToken, name?}` → `{connectionId, accounts: WireRemoteAccount[]}` or `WireProblem` error (HTTP 422 with `{problem}`) |
| POST   | `/api/sync/connections/{id}/token`           | `{setupToken}` → same as connect (keeps the id and links)                        |
| GET    | `/api/sync/connections/{id}/accounts`        | → `{accounts: WireRemoteAccount[]}` (balances-only fetch, counts quota)          |
| DELETE | `/api/sync/connections/{id}`                 | → `WireSyncStatus` (removes the keychain item and the `[[sync.connection]]` entry; accounts stay and read as not-connected) |
| GET    | `/api/sync/connections/{id}/diagnostics`     | → `text/plain`, redacted                                                         |
| POST   | `/api/sync/share/preview`                    | `{manualRulesId, journalAccount}` → `WireSharePreview`                           |
| POST   | `/api/sync/link/preview`                     | `{links: WireLinkRequest[]}` → `{accounts: WireLinkPreview[]}` (dry run only; fetches unless a fresh batch from connect/accounts is cached < 10 min) |
| POST   | `/api/sync/link`                             | `{links: WireLinkRequest[]}` → `WireSyncRun` (writes rules, shares, `[sync]`, account declarations, opening balances, then the first sync) |
| POST   | `/api/sync/now`                              | `{connectionId?, accountKey?}` → `WireSyncRun`                                   |
| PATCH  | `/api/sync/accounts/{key}`                   | partial `{journalAccount?, target?, rules?, startDate?, balanceSign?, enabled?}` → `WireSyncStatus` |
| DELETE | `/api/sync/accounts/{key}`                   | → `WireSyncStatus` (stop syncing; history stays)                                 |
| POST   | `/api/sync/held/{key}/{heldId}`              | `{action: "import" \| "duplicate"}` → `WireSyncStatus`                           |
| GET    | `/api/sync/manual`                           | → `{accounts: WireManualAccount[]}`                                              |
| PUT    | `/api/sync/manual`                           | `WireManualAccount` (account, downloadUrl?, nudge) → `{accounts: WireManualAccount[]}` |

### Wire shapes (camelCase JSON; times are epoch seconds; dates are ISO `YYYY-MM-DD`; money is decimal text)

```ts
type Who = 'bank' | 'simplefin' | 'ledgeline' | 'network' | 'setup';
type Severity = 'info' | 'warning' | 'error';
type ProblemCode =
  | 'con-auth' | 'act-failed' | 'revoked' | 'payment' | 'quota' | 'network' | 'edge-blocked'
  | 'token-used' | 'stale' | 'hledger' | 'rules' | 'held' | 'balance' | 'target'
  | 'not-connected' | 'setup' | 'other';
interface WireProblem {
  code: ProblemCode; severity: Severity; who: Who;
  message: string;                 // plain sentence, ready to show
  since: number | null;
  dataThrough: string | null;      // "Data stops at Nov 11"
  action: null | {
    kind: 'open-url' | 'reconnect' | 'retry' | 'edit-rules' | 'review-held' | 'choose-file'
        | 'connect' | 'set-hledger' | 'show-balance' | 'recheck';
    label: string; url?: string;   // url for open-url; rulesId etc. in `ref`
    ref?: string;
  };
  secondary: null | { kind: 'recheck'; label: string };   // "I've fixed it"
  detail: string | null;           // raw code + server message, sanitized
  docAnchor: string;               // e.g. "con-auth" → docs/imports/bank-sync.md#con-auth
}
interface WireSyncStatus {
  autoSync: boolean; available: boolean; unavailableReason: string | null;
  running: boolean; lastRun: number | null; nextAuto: number | null;
  connections: WireConnection[]; accounts: WireLinkedAccount[]; unlinked: WireRemoteAccount[];
  problemCount: { warning: number; error: number };
}
interface WireConnection {
  id: string; name: string; kind: 'simplefin'; host: string | null;
  connected: boolean;             // keychain item present on this machine
  state: 'ok' | 'syncing' | 'attention' | 'broken' | 'not-connected';
  lastAttempt: number | null; lastSuccess: number | null;
  requestsToday: number; quota: { autoMax: number; manualWarn: number; manualMax: number };
  manageUrl: string;               // https://beta-bridge.simplefin.org/ for the Bridge, else https://<host>/
  problems: WireProblem[];
}
interface WireLinkedAccount {
  key: string; connectionId: string; remoteId: string; label: string; org: string | null;
  journalAccount: string; target: string; rules: string; categories: string | null;
  startDate: string; balanceSign: 1 | -1; enabled: boolean;
  lastSuccess: number | null; dataThrough: string | null; lastAdded: number;
  bankBalance: { amount: string; date: string } | null;
  ledgerBalance: string | null; balanceDiff: string | null;   // null when equal or unknown
  held: WireHeld[];
  history: { at: number; outcome: 'added' | 'nothing-new' | 'failed' | 'skipped'; added: number; held: number }[];
  problems: WireProblem[];
}
interface WireHeld {
  heldId: string; date: string; amount: string; payee: string;
  existing: { index: number; date: string; description: string; amount: string };
}
interface WireRemoteAccount {
  connectionId: string; remoteId: string; label: string; org: string | null;
  balance: string; balanceDate: string | null; currency: string;
  oldestAvailable: string | null;                 // earliest txn date seen in the fetch
  suggestion: {
    journalAccount: string | null; score: number; ledgerBalance: string | null;
    signFlipped: boolean; startDate: string; target: string | null;
    newAccountName: string;                       // e.g. "assets:bank:ally:savings"
    newAccountType: 'A' | 'L' | 'C';
    syncRules: { id: string; fits: boolean }[];   // existing sync-shaped files
    manualRules: { id: string; ruleCount: number } | null;   // a manual file covering the account
  };
}
interface WireLinkRequest {
  connectionId: string; remoteId: string; label: string;
  journalAccount: string;
  newAccount: null | { type: 'A' | 'L' | 'C'; openingBalance: string; openingDate: string };
  startDate: string; target: string; balanceSign: 1 | -1;
  rules: { mode: 'new'; name: string } | { mode: 'existing'; id: string };
  share: null | { manualRulesId: string };        // run the categories refactor first
}
interface WireLinkPreview {
  remoteId: string; ok: boolean; error: WireProblem | null;
  new: number; already: number; held: number; uncategorized: number;
  droppedBeforeStart: number; entries: string;    // proposed journal text
}
interface WireSharePreview {
  manualRulesId: string; categoriesId: string; moved: number;
  kept: { header: string; reason: string }[];
  diffs: { manual: string; categories: string };  // unified diffs
  identical: boolean; differences: string | null; // hledger print before/after check
}
interface WireSyncRun {
  ok: boolean; requestsToday: number; quotaWarning: boolean;
  accounts: { key: string; label: string; added: number; held: number; already: number;
              outcome: 'added' | 'nothing-new' | 'failed' | 'skipped'; problem: WireProblem | null }[];
  problems: WireProblem[];
}
interface WireManualAccount {
  account: string; lastTransaction: string | null; lastImport: string | null;
  downloadUrl: string | null; nudge: boolean; stale: boolean;   // stale = nudge && >35 days
}
```

Error mapping: the plan's §6 table, one `ProblemCode` per row.

---

## WP-W: web

- **Placement:**
  - Imports gets a first tab, **Accounts** (keys 1–3; existing tab ids and
    deep links keep working).
  - Settings gets a **Sync** tab.
  - The header gets a status dot (hidden when there are no links), linking
    to `/imports?tab=accounts`.
  - Sync problems are merged into `ProblemsBadge`/drawer, labelled "Bank
    sync".
  - Native (iOS): the Accounts tab and the Sync settings tab are hidden, as
    Prices is today.
- **Store:** `web/src/lib/sync/syncStore.svelte.ts` polls
  `/api/sync/status` every 60 s while visible (reuse `visiblePoll`), and
  every 2 s while `running`.
- **Pure helpers** go in `.ts` files with tests:
  - `statusLine`
  - `problemAction`
  - `wizardModel` (step state machine, token pre-validation by base64 decode
    in the browser)
  - `manualStale`
- **Components** go in `web/src/lib/sync/ui/`:
  - `AccountsPanel`
  - `ConnectionCard`
  - `LinkedAccountRow`
  - `ManualAccountRow`
  - `AccountDrawer`
  - `ConnectWizard` (steps: choose, simplefin, link, preview)
  - `ProblemNotice`
  - `HeldList`
  - `SharePreviewDialog`
  - `SyncNowButton`
  - `SyncStatusDot`
  - `SyncSettingsPanel`
- **Fixtures:** component tests use fixtures in
  `web/src/lib/sync/fixtures.ts` that satisfy the types above. Build all UI
  against them. The server lands later.
- **Copy:** the plan's wording (§4, §6) is the copy to use. Follow the house
  style, which is short, plain sentences.
- **"Who sees your data"** is an expandable table in the wizard and on each
  connection card's ⓘ.
- **Clipboard:** offer "Paste token from clipboard" only when the clipboard
  read succeeds on focus. Never read it silently in the background.

---

## Deviations

(Each WP appends here, with the reason.)

- **Plan §4.3 step 4, "Review now".** The wizard previews counts and the
  proposed text (`WireLinkPreview.entries`). **Finish** writes. "Review now"
  then opens Journal → Review, filtered to unreviewed, over what was just
  written. Writes on click were Patrick's decision #1, and the card review
  works over the journal anyway. This avoids building a second staged-review
  host.
- **Sync never reports `conflicting` rows.** A known id is always
  `already`, because a hand edit wins. Only `held` (a possible duplicate)
  asks the user.
- **WP-A: `TokenError::NoCredentials`.** `AccessUrl::parse` needs a fifth
  variant for an access URL with no username or password. None of the four
  listed ones fit. Match it non-exhaustively.
- **WP-A: `Normalized::dropped_invalid`.** This is an extra counter for rows
  whose amount isn't a plain decimal or whose id is empty once made
  tag-safe. `parse_accounts` already drops and reports those, so the counter
  is non-zero only for a `FeedAccount` built by hand, such as a fake provider.
- **WP-A: a bad balance is `""`.** `FeedAccount::balance` stays a `String`.
  A missing balance, or one that isn't a plain decimal, becomes `""` plus an
  `Other` problem, and the account is kept. WP-S treats `""` as "unknown".
- **WP-A: how the `/accounts` reader behaves.**
  - When `errlist` is present, `errors` is ignored, so nothing is reported
    twice.
  - v1 yields no `connections`. Accounts carry `org_name`/`org_url` from
    `org`, and `org.domain` becomes `https://<domain>/`.
  - Server URLs (`org_url`) are kept only when they're `http(s)`.
  - The money check is stricter than `Dec::parse`: `-?digits(.digits)?`.
- **WP-A: quota text.** A message counts as `Quota` when it mentions
  "quota", or "limit" without "range", so `gen.api`'s 90-day *range* cap
  isn't counted. The rule applies to v1 strings and to `gen.*` or unknown v2
  codes, never to `con.*`/`act.*`.
- **WP-A: transaction text is cleaned.** `normalize` collapses whitespace
  and strips control and bidi characters from payee, description and memo.
  A newline in a CSV field would land inside a journal description. `to_csv`
  still quotes every field correctly (property-tested with the `csv` crate).
- **WP-A: how dupes match.**
  - The amount matches a posting with exactly **one** amount, in any
    commodity, because the feed's `USD` is the journal's `$`. Auto-generated
    postings are ignored.
  - A row claims the nearest-dated unclaimed match; ties go to file order.
  - `Existing::index` is the 1-based `Tindex`.
- **WP-A: config validation.**
  - Connection ids must be 1–64 of `[A-Za-z0-9_-]`, because they become the
    keychain key.
  - `kind` must be 1–32 of `[a-z0-9-]`. It defaults to `simplefin`, and
    `ConnectionLink::kind` is a `String`.
  - A `downloadUrl` that isn't `http(s)` is dropped with a warning; the
    entry is kept.
  - Writers apply the same checks and return `ConfigError::Invalid` rather
    than write something `parse` would skip. Other errors are `InvalidToml`
    and `Shape`.
- **WP-A: `reimport` now has `pub fn ids_of`.** It returns every id on a
  transaction, deduplicated. `build_index` uses it. `id_of` and `classify`
  are unchanged.
- **WP-R: `append_include` returns `Result<String, ShareError>`.** It
  validates the name with the new public `share::check_include_name`: a
  plain sibling `*.rules` name, with no separators, whitespace, control or
  glob characters. A bare `String` return would let a bad name put a second
  rule into the file.
- **WP-R: `SharePlan` has two extra fields, `moved_spans: Vec<Span>` and
  `include_offset: usize`.** They are what the byte-identity property test
  reassembles from. `ShareError` is `BadFileName | NoFieldsLine |
  AlreadyShared(String) | Unsafe`. `Unsafe` is a re-parse guard and should
  never fire.
- **WP-R: there's a fourth keep reason, `KeepReason::OrderDependent(field)`.**
  The include goes last, and in hledger the last match wins. A block that came
  *before* a kept block assigning the same field would start beating it after
  the move (verified on 1.52). So that block stays too. This applies
  transitively, and an `include` of some other file counts as a wall. The rule
  is strict: one kept `account2` rule late in a file keeps every earlier
  `account2` rule. `KeepReason` implements `Display` with a plain sentence for
  `WireSharePreview.kept[].reason`.
- **WP-R: `LayoutDependentPattern` also covers two more cases.** One is a
  whole-record pattern containing `"`, since CSV quoting differs between the
  files. The other is any matcher on the date column when the manual file's
  `date-format` isn't `%Y-%m-%d`, because sync dates are always ISO.
- **WP-R: a block whose assignment values interpolate a column sync lacks
  stays, as `FieldNotInSync(name)`.** For example, `comment fitid:%fitid`.
  hledger reads an unknown `%name` as empty without an error.
- **WP-R: the feed template assigns `date2` inside `if %transacted .`.** The
  contract's unconditional `date2 %transacted` is a hard hledger error on an
  empty cell ("could not parse "" as a date"), verified on 1.52. The template
  also adds `date %X` / `amount %X` when those column names are borrowed and
  aren't `date` / `amount`.
- **WP-R: the matching skip.** `matching::reads_csv_alone` returns true for a
  file with a `fields` line or with any `include`, because `fields` may come
  from an included file (`fixtures/review-rules/import/paypal.csv.rules`).
  `prefilter` checks it, so `import_api::rank_candidates` gets the skip
  without any server edit. A categories file has neither, so it's never a
  candidate.
- **WP-R: the `FeedColumns` mirror.** It lives in `rules/generate.rs`, with
  the same fields, order and defaults, plus an `in_order() -> [&str; 7]`
  helper that `feed_rules` uses. It carries a MERGE SEAM comment. On merge,
  replace it with a re-export of `feeds::normalize::FeedColumns`, and either
  add `in_order()` there or inline it.
- **WP-R finding (resolved by WP-S, see below): shared `comment` rules erase the sync
  id.** A conditional assignment beats a top-level one. So any categories
  rule that sets `comment` replaces the template's `comment id:%id` on the
  sync rows it matches, and those rows land with no `id:` tag. Then
  `reimport` can't dedup them; see `split/firsttech-visa.sync.csv`, row
  SHELL. One option: emit `if %id .` / `comment id:%id` *after* the include,
  so ids win and category comments are lost on sync rows. Another: keep
  comment-assigning blocks out of the categories file. Either way it needs a
  decision.
- **WP-R finding (resolved by WP-S, see below): the feed template has no
  `currency`.** Synced amounts come in bare, while a manual file usually says
  `currency $`. That's matching.rs's "commodity trap". A `currency` field on
  `FeedRulesParams`, fed from SimpleFIN's account currency, would fix it.

### From WP-W (web)

Requests for WP-S:

- **`WireSyncRun.accounts[].uncategorized?: number`** (added, optional): new
  rows that landed on the fallback account. The Sync now toast's
  "Review 4" uses it; without it the toast falls back to `added`. The
  decoder accepts it when present.
- **Sync problems are merged into the Problems badge/drawer on the client**,
  from `/api/sync/status`. Don't also add them to `/api/diagnostics`, or
  they count twice.
- **ui-prefs:** the SPA decoder defaults the four sync keys when absent, so
  it works against today's engine. When `ui_prefs.rs` gains them, add them
  (with defaults) to the golden `fixtures/native/v1/ui-prefs.json`. The
  SPA's `PINNABLE` order is theme, uiScale, autoPrices, autoPricesWeeks,
  autoSync, autoSyncEveryHours, syncSourceDir, syncKeepSourceDays.
- **422 `{problem}`** is parsed from the raw body (no length cap), on every
  sync write route.
- **`action.ref`:** the UI reads it as a connection id for `reconnect`, an
  account key for `review-held` / `choose-file` / `show-balance`, and a rules
  id for `edit-rules`. Without `ref` it uses where the problem was listed
  (the connection's or account's `problems`).
- **`open-url`** is rendered only for `https://` URLs without credentials;
  anything else drops the button. `manageUrl` falls back to the Bridge.
- **Docs anchors:** details link to
  `https://zmre.github.io/ledgeline/imports/bank-sync/#<docAnchor>` (WP-D).
- **e2e fake host:** the browser token check accepts any `https://…/claim/…`
  URL; `fake.simplefin.test` shows the "Not SimpleFIN Bridge" warning but
  can connect. Bridge hosts: `beta-bridge.simplefin.org`,
  `bridge.simplefin.org`.

UI decisions that differ from the plan:

- **Imports default tab** stays New Transactions for a bare `/imports`
  (existing links and e2e). Accounts is first in the strip, key 1.
- **No "Rename" on a connection:** there is no route for it. If wanted:
  `PATCH /api/sync/connections/{id}` `{name}` → `WireSyncStatus`.
- **Share prompt** offers Share them · Keep separate. "Start the sync rules
  empty" had no wire difference from Keep separate.
- **Auto-sync toggle** sits on the preview step and is saved (ui-prefs) only
  after `/api/sync/link` succeeds.
- **Drawer → Change…** takes the rules file as journal-relative text, because
  `WireLinkedAccount` has no list of fitting sync rules files. Optional
  request: `rulesChoices: {id, fits}[]` on it.
- **Not yet wired:** "Import a file…" opens New Transactions without
  presetting the account, and the drawer's rules **Edit** opens Edit Rules
  without selecting the file. Neither screen takes that from the URL today.
- **Dev only:** `localStorage["ledgeline.dev.syncFixtures"] = "1"` in a dev
  build serves `web/src/lib/sync/fixtures.ts` instead of the engine. A
  production build drops it.

### WP-S (server)

Lead decisions of 2026-11-15, carried out:

- **Merge seam.** `rules::generate::FeedColumns` is now a re-export of
  `feeds::normalize::FeedColumns` (which has `in_order()`); WP-R's mirror is
  gone.
- **Shared `comment` rules can't erase sync ids — fixed both ways.**
  `share::KeepReason::SetsComment` ("Sets the transaction comment, which sync
  uses for its id.") keeps a block that assigns the transaction `comment`
  (not `comment1`…) in the manual file. And `feed_rules` ends every sync file
  with an **id guard**, `if %<id> .` / `  comment id:%<id>`, after the include,
  so it is the last conditional and always wins. The top-level `comment
  id:%id` stays for readability. WP-R's tests and goldens were updated
  (`wells-ofx` now moves 6 blocks, keeps VENMO; `firsttech-visa` keeps its two
  comment blocks); `just rules-check` and `LEDGELINE_HLEDGER_SHARE_CHECK=1`
  pass, with a new hledger test proving the guard beats a categories comment.
  When sharing adds an include to an existing sync file, the server puts it
  *before* the guard.
- **Currency.** `FeedRulesParams.currency: Option<&str>` emits `currency X`
  after `date-format`. The server picks: the commodity most used on the
  linked account; else `$` for `USD` when the journal is mostly `$`; else the
  ISO code. A custom-currency URL is fetched (no credentials) only when its
  scheme and host are the access URL's (`FeedProvider::currency_symbol`,
  letters only, ≤ 12); otherwise the symbol is `?` and a warning problem.
- **No `conflicting`.** Rows are split with `dupes::split` over the whole
  journal's ids *before* staging; only `fresh` rows go into the CSV.
- **"Review now"** is the web's; no staged review host server-side.

WP-W's requests, implemented:

- `WireSyncRun.accounts[].uncategorized`: written rows posted to the rules
  file's `account2` (default `expenses:unknown`), counted from the dry run's
  entries.
- Sync problems are **not** added to `/api/diagnostics`.
- The four keys are in `fixtures/native/v1/ui-prefs.json`.
- `action.ref`: connection id for `reconnect`/`connect`; account key for
  `review-held`, `choose-file`, `show-balance`; rules id for `edit-rules`.
  `docAnchor` is the problem code (`con-auth`, `revoked`, …, `other`).
- `PATCH /api/sync/connections/{id}` `{name}` → `WireSyncStatus`, and
  `WireLinkedAccount.rulesChoices: {id, fits}[]`.

Other deviations:

- **`POST /api/sync/connect` writes `[[sync.connection]]` straight away**,
  not at link time: the keychain item exists from the claim on, and an entry
  with no accounts is what lets "Re-check" list it later and Disconnect
  remove the key. The answer may carry `problem` beside `connectionId` and
  `accounts` when the claim worked but the listing didn't (or found no
  banks: an `info` "SimpleFIN has no banks connected yet…").
- **Errors.** A wizard problem is `422 {problem}`; a write on a read-only
  server is `403` with a plain-text sentence; everything else is the crate's
  usual plain-text `AppError` status.
- **Link preview cache.** A connect/accounts batch is balances-only, so it
  can't preview transactions. The preview caches its own transaction fetch for
  10 minutes instead (same connection, reaching back far enough, holding the
  accounts asked for).
- **Sharing places the new sync file beside the manual file** (an `include`
  names a sibling); an existing sync file must already be in that folder.
  When the manual file has no saved CSV to run, the byte-identity check can't
  run and the split is trusted (`identical: true`): it is a pure byte
  partition, property-tested in core.
- **New accounts** get `account NAME  ; type:X` plus `YYYY-MM-DD Opening
  balance` / `equity:opening balances` in the link's **target** file (plan §11
  open question, decided).
- **Source folders:** `<syncSourceDir>/<slug(connection name)>/<slug(label)>-<key[..6]>/<YYYY-MM-DDTHHMM>.csv`,
  local time, `-2`… for a second sync in the same minute. `syncSourceDir` is
  resolved through `vault::canonicalize_existing_prefix` *before* anything is
  created, so a symlink out of the journal folder is refused.
- **No raw responses on disk.** Diagnostics are built from `sync-state.json`
  (last 10 outcomes, the last error list, problems), credential-scrubbed by a
  new `redact::scrub_credentials` and path-redacted. No bank response body is
  written anywhere.
- **`sync-state.json`** also remembers each connection's remote accounts
  (balances only) for the "not linked" rows, the host for display, and
  `failingSince` for "since…" and act.failed's 3-day escalation. The quota is
  counted and saved *before* each request is sent.
- **Thresholds:** `stale` after 14 days without new data (severity `info`);
  network problems are `info` for 24 h, then `warning`; a manual account is
  stale after 35 days; `lastImport` is always `null` (there is no import log to
  read yet).
- **Suggestions** use their own scoring (last four digits in the account
  name, shared name words, a balance equal or negated), not
  `qb_import::suggest_accounts`, which takes a `QbJournal`.
- **Core additions outside WP-S's files:** `EditOp::AddTxnTagValue` /
  `JournalEditor::add_txn_tag_value` in `edit.rs` ("It's a duplicate" adds a
  second `id:` and keeps the first; there was no transaction-level op for
  that); `import_api` gained `HeadlessImport`, `stage_headless`,
  `headless_dry_run`, `headless_commit` (thin wrappers over `run_dry_run` /
  `run_commit`, like `run_cli_import`).
- **`FeedProvider`:** `claim(&SetupToken) -> access URL text`,
  `accounts(&AccessUrl, &AccountsQuery)`, `currency_symbol` (default `None`).
  `filter_batch` is public for fakes.
- **Keychain:** `keyring` 3.6 (4.x splits into `keyring-core` + stores), per
  target: `apple-native`; Linux `async-secret-service` + `crypto-rust` +
  `async-io` (pure Rust zbus, no openssl/libdbus/native-tls); `windows-native`.
  Behind a new default server feature `keychain`; without it (iOS/FFI) the
  store is `UnavailableStore` and connecting is refused. `chrono` (`clock`) is
  added for the local UTC offset.
- **Test seams (`test-fakes`):** `with_sync_loopback_http`, `with_fake_feed`,
  `inject_sync_conflicts`; the server's own tests get the feature through a
  dev-dependency on the crate itself. Without the feature,
  `LEDGELINE_FEED_FAKE` is ignored with a warning. `just e2e` and
  `just docs-screenshots` build with it; **the Playwright config must still set
  `LEDGELINE_FEED_FAKE` for the engine it starts, and the sync specs should run
  on a copy of the fixture ledger** — linking writes `ledgeline.toml`, rules
  files and CSVs beside the journal.
- **hledger-backed engine tests** are gated by `LEDGELINE_HLEDGER_SYNC_CHECK=1`
  (added to `just hledger-checks`), like the other hledger suites.
- **CLI:** `ledgeline sync -j <journal> connect|accounts|link|status|now`,
  printing each route's JSON, plus `reconnect <connection> <token>` (the
  token-replacement route) and `diagnostics <connection>` for real-bank QA.
- **The scheduler** is spawned by the desktop app and `--server`, but not
  under `LEDGELINE_UI_PREFS_IN_BROWSER` and never by the FFI.

