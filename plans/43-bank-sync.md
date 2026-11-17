# Plan 43: Bank sync (SimpleFIN, with room for Plaid)

Status: **draft for Patrick's review.** Decisions from his first read
(2026-11-15) are folded in and listed in §10. Nothing is built yet.

## Goal

The user connects a bank once. After that, new transactions show up in the
journal: categorized by rules, deduplicated, and waiting in the Review queue.
It should work like automatic price updates (plan 41):

- a **Sync now** button;
- an **auto-sync** setting that fetches the most recent end-of-day data at
  startup, and again periodically when the app stays open for days.

Banks without a connection keep working through manual downloads, and the
same screen shows both kinds side by side. When something breaks, the user
is told what happened, whose side it's on, and given one button that fixes it.

---

## 1. Research findings that shape the design

The full research, with sources, is in the appendix. These are the parts
that change the design.

### 1.1 "SimpleFIN direct" doesn't exist in practice

The SimpleFIN spec says banks ought to run their own servers, but none do.
The ecosystem page lists no bank servers, and the Bridge's own mission
statement is "to not exist". The only SimpleFIN servers are aggregators:

| Server                 | Region          | Behind it                       | Price            |
| ---------------------- | --------------- | ------------------------------- | ---------------- |
| SimpleFIN Bridge       | US              | MX                              | $15/yr           |
| Synci                  | UK / EU / NZ    | PSD2 open banking               | paid             |
| Redbark                | AU / NZ / US    | own aggregator                  | A$16/mo          |

**Consequence:**

- The SimpleFIN client accepts **any** server root that issues a setup token.
  If a bank ever runs its own, it works with no new code.
- The UI never promises a direct SimpleFIN path. It names who sees the data.

### 1.2 The no-third-party path is OFX Direct Connect, and it's fragile

This is what Quicken, Moneydance and GnuCash call "Direct Connect": the
bank's own OFX server. Its state as of this month:

| Institution                           | State                                          |
| ------------------------------------- | ---------------------------------------------- |
| Fidelity                              | Dead. The host was removed from DNS in 2026.   |
| Schwab                                | Dead since 2020.                               |
| Chase                                 | Host doesn't resolve.                          |
| Vanguard                              | Host resolves; requests time out.              |
| Amex, Wells Fargo, Discover, USAA, E*Trade | Hosts answer. Not proven to serve OFX.    |
| Many credit unions                    | Still listed in the GnuCash and ofxtools directories. |

Other problems:

- The ofxhome.com directory is gone.
- Many banks reject any client that doesn't pretend to be Quicken.
- Logins that need MFA or a one-time identity check can't be automated.

**Consequence (decided 2026-11-15):** Direct Connect is **not planned**.
The wizard doesn't mention it. The provider abstraction still lets us add
an individual bank's own direct API later, if one is worth it. Users who
won't accept a third party get manual downloads, and the wizard explains
the tradeoff.

### 1.2b Going straight to an aggregator with your own keys (researched 2026-11-15)

Patrick asked whether people could deal with MX directly (since MX is
underneath SimpleFIN), or with Stripe. The short answer:

- **MX: no.** A free developer account is self-serve, but production access
  (real OAuth banks) means a sales contract, IP whitelisting, and no public
  pricing or personal tier.
- **Stripe Financial Connections: probably not.**
  - It's built for a business collecting its *customers'* accounts. Live
    transactions need a registration step, and nothing in Stripe's docs says
    a person linking their own accounts would be approved or permitted.
  - The data is thin: no merchant name or category.
  - Its refresh notice comes by webhook (polling works).
  - It's US-only.
  - **Cost** is $0.30 per institution per month for transactions, plus $0.10
    per balance call. About $22/yr for 6 banks, but daily balance checks on
    10 accounts would add about $365/yr.
  - It uses Finicity and MX underneath.
- **Plaid Trial: yes, and this is new.**
  - Since 16 Apr 2026 Plaid offers a free, self-serve Trial plan, described
    as "more user-friendly for non-business users". It needs no business
    registration or sales process, only a short questionnaire.
  - Since 20 May 2026 it includes all OAuth banks (Chase, BofA, Wells,
    Capital One, Citi).
  - **The catch:** a lifetime cap of **10 Items**. An Item is one bank
    login, not one account, and the cap counts every login ever linked, so
    re-linking a broken connection uses another slot. Past that, Plaid's
    pay-as-you-go plan has no minimum spend but unpublished prices, estimated
    at $0.25–0.60 per login per month.
  - **Keys:** the user's own client id and secret live in their keychain.
  - **Linking:** Plaid's JS widget runs in our webview. OAuth banks open in
    a popup, so no redirect URL and no Ledgeline server are needed.
    *Unverified:* that the popup works inside wry's WKWebView.
- **Teller: possible, but fragile.**
  - The developer tier is free up to 100 linked logins. There's a JS widget
    plus a client certificate that lives on the user's machine.
  - It's widely described as using reverse-engineered bank access rather
    than bank OAuth, so expect breakage. Its terms on personal use weren't
    confirmed.
- **Outside the US:**
  - Enable Banking (EU): a free "restricted production" mode limited to your
    own accounts.
  - Akahu (NZ): free personal apps.
  - GoCardless/Nordigen: closed to new signups since July 2025.

| Provider            | Individual self-serve?           | ≈ annual cost, ~10 accounts      | Needs our server? | US OAuth banks         | Who sees your data       |
| ------------------- | -------------------------------- | -------------------------------- | ----------------- | ---------------------- | ------------------------ |
| SimpleFIN Bridge    | Yes                              | $15                              | No                | Yes, via MX            | SimpleFIN + MX           |
| Plaid Trial         | Yes (US/CA, short questionnaire) | $0 up to 10 logins, ever         | No                | Yes                    | Plaid                    |
| Plaid pay-as-you-go | Yes                              | ≈ $30–70 (unofficial)            | No                | Yes                    | Plaid                    |
| Teller (dev tier)   | Yes                              | $0 up to 100 logins              | No                | Claimed, not via OAuth | Teller                   |
| Stripe FC           | Business registration, unclear   | $22 + balance calls              | No (polling)      | Yes                    | Stripe + Finicity/MX     |
| MX direct           | No (sales contract)              | not public                       | n/a               | after approval         | MX                       |

**What it changes:**

- **SimpleFIN stays first.** It's the least work for the user: one website
  and one pasted token. It's cheap, has no lifetime cap, and has no developer
  dashboard or questionnaire to get through.
- **Plaid Trial with the user's own keys is the second provider (Phase 4).**
  It's free for most households, removes MX from the picture, and its data
  is richer: merchant names, and links from pending to posted.
- **The cost is a more technical setup:**
  - create a Plaid account;
  - answer the questionnaire;
  - copy the client id and secret;
  - then link banks inside Ledgeline.

  The wizard would present it as "Advanced: use your own Plaid account".
- **A relay server we'd run is no longer needed for Plaid.**
- **Stripe, MX and Teller are not planned.**

### 1.3 SimpleFIN Bridge facts that constrain the engine

- **Rate limit:** ≤ 24 requests per day per token. Going over first adds
  warnings to the response, then gets the token disabled. Requests should
  start at a jittered minute.
- **Window:** ≤ 90 days per request (longer ranges are silently capped and a
  `gen.api` error is added). The Bridge filters by day only and isn't clear
  which date it filters on, so requests should overlap about 5 days and the
  client should filter locally.
- **Data freshness:** MX refreshes about once a day. Fetching more often
  than every few hours gains nothing.
- **Ids aren't reliable:**
  - Ids are unique only *within an account*.
  - Pending and posted versions of a transaction can have different ids and
    amounts. Actual's users ended up double-counting because of this.
  - Capital One re-issued whole blocks of ids, which duplicated transactions
    in client apps.
- **Errors (v2):** an `errlist` of `{code, msg, conn_id?, account_id?}`.
  - `con.auth` means the bank connection needs re-linking on the Bridge site.
  - `act.failed` means one account failed.
  - HTTP 402 means the subscription has lapsed.
  - HTTP 403 means the token was revoked or disabled.
  - A 403 when claiming a setup token means it was already used, which may
    mean the token was intercepted.
- **Cloudflare:** the Bridge returns 403 at the edge for unrecognised User-Agents.
  That looks exactly like "token already used", so we send our own User-Agent
  and use the `cf-ray` header to tell the two cases apart.
- **Demo server:** `…/claim/DEMO` can be claimed repeatedly, and its access
  URL is `https://demo:demo@beta-bridge.simplefin.org/simplefin`. It has 3
  accounts and no pending items. It's good for an opt-in live smoke test; it
  isn't enough for CI.

### 1.4 What Ledgeline already has

| Need                         | Existing piece                                                                                  |
| ---------------------------- | ----------------------------------------------------------------------------------------------- |
| Statement → journal          | Pipeline A: `Tabular` CSV → `hledger import` + rules → dry run → review → commit (`import_api.rs`) |
| Dedup by bank id             | `reimport.rs`: `comment id:%fitid` gives new / unchanged / statusChanged / conflicting           |
| Categorization               | rules files; "Remember as a rule" from review (plan 32)                                         |
| Review                       | `reviewed:` tag and the Journal Review queue (plan 32)                                          |
| Transfer pairing             | plan 35 `match:` tags on clearing accounts                                                      |
| Background schedule          | `auto_prices.rs`: engine Tokio task, 5-min wall-clock wake, pure planner, backoff, polled status |
| Settings, pinnable per ledger | `ui_prefs.rs` `PINNABLE`                                                                        |
| Attention surface            | `ProblemsBadge` / `/api/diagnostics`                                                            |
| HTTP client                  | `reqwest` + rustls, `yahoo::http_client()`                                                      |
| **Secret storage**           | **None.** New.                                                                                  |
| **Source account → journal account mapping** | **None.** Rules files are matched again on every upload. New.                    |

iOS hides Imports because it needs the hledger subprocess. **Bank sync is
desktop-only in this plan.**

---

## 2. Concepts and vocabulary (what the user sees)

- **Connection:** one way of reaching one or more institutions. Examples:
  "SimpleFIN Bridge" (one token covers every bank linked there), or
  in future, a single bank's own API.
- **Linked account:** one account at the bank, mapped to one journal account.
- **Manual account:** a journal account the user imports by downloading
  statements. It's listed beside linked accounts with its last import date
  and a link to the bank's download page.
- **Sync:** fetch → save the source file → import through the rules → dedup →
  write to the journal as unreviewed.

The UI always says **who sees your bank data** for each connection:

| Connection type       | Who sees your transactions                       | Where your bank password lives |
| --------------------- | ------------------------------------------------ | ------------------------------ |
| SimpleFIN Bridge      | SimpleFIN Bridge and MX (its data provider)      | With MX, not with Ledgeline    |
| Manual download       | Nobody but you                                   | Nowhere (you log in yourself)  |

---

## 3. Architecture

### 3.1 Provider abstraction (`ledgeline-core::feeds`)

The provider abstraction is pure and has no I/O; the network lives in
`ledgeline-server`.

```rust
pub enum ProviderKind { SimpleFin /*, Plaid (own keys), a bank's own API */ }

pub enum ConnectFlow {
    PasteToken,        // SimpleFIN: setup token → claim → access URL
    OwnKeysWidget,     // Plaid (Phase 4): user's client id/secret + Plaid Link in the webview
}

pub enum FetchMode { DateWindow { max_days: u32 }, Cursor }

/// One fetch's result, normalised across providers.
pub struct FeedBatch {
    pub accounts: Vec<FeedAccount>,   // id, name, org, currency, balance, balance_date, extra
    pub txns: Vec<FeedTxn>,           // account_id, id, posted, transacted, amount, payee,
                                      // description, memo, pending_link: Option<id>
    pub problems: Vec<FeedProblem>,   // normalised errlist (see §6)
    pub notices: Vec<String>,         // x-api-message etc., sanitised
}
```

In `ledgeline-server`:

```rust
#[async_trait]
trait FeedProvider: Send + Sync {
    async fn fetch(&self, secret: &Secret, window: Window, accounts: &[AccountId])
        -> Result<FeedBatch, FeedError>;
    async fn list_accounts(&self, secret: &Secret) -> Result<FeedBatch, FeedError>; // balances only
}
```

- It's swappable on `AppState`, the same way `PriceFeed` is. Tests inject a
  fake.
- Plaid (with the user's own Trial keys, §1.2b) would be a second
  implementation. It needs no server of ours. The concessions to it are:
  - the `ConnectFlow` variant;
  - no provider assumes a date window (Plaid's `/transactions/sync` uses a
    cursor, and the cursor is machine state for `sync-state.json`);
  - a provider may report "removed" transactions, which go to "Needs a
    look", never auto-deleted.

### 3.2 Normalization → existing Pipeline A

Each linked account's batch becomes a `Tabular` with fixed columns:

```
date, amount, payee, description, memo, id, transacted   (payee renamed to match a shared manual file, below)
```

- `date` is the bank's posted date (decided). `transacted` becomes
  hledger's secondary date (`date2 %transacted` in the rules file), but only
  when it differs from `posted`. Otherwise the column is empty and hledger
  writes no secondary date.
  - Open item: a midnight-UTC epoch is read as that UTC date; any other
    timestamp as a local date. Verify against real Bridge data in Phase 1.
- `id` is `<acct-short>-<txid>`, made tag-safe (commas stripped, like
  `ofx::tag_safe`). The prefix matters: SimpleFIN ids are unique only within
  an account, so two accounts' ids could collide in one journal. `acct-short`
  is 6 hex characters from a hash of (connection, account id).
- `payee` falls back to `description` when it's missing.
- Zero-amount rows are dropped, as Firefly does.

#### Rules files: chosen at setup, changeable later, categorization shared

**The user picks each account's rules file at link time** (§4.3 step 3).
They can switch it later from the account drawer. The choices:

- **Create a new sync rules file** (the default): `firsttech-checking.sync.csv.rules`
  is generated by `rules/generate.rs`, which gains a "feed" template.
- **Use an existing sync rules file.** This list only shows files whose
  `fields` line fits the sync CSV shape. Fit is scored with the existing
  `rules/matching.rs` against a sample sync CSV. A manual-download rules
  file (OFX's `name, trntype, fitid…`, or a bank's own CSV columns) never
  fits, which is why the next point exists.

```
# ledgeline: sync rules for SimpleFIN account "Checking ••1234" (First Tech)
skip 1
fields date, amount, name, description, memo, id, transacted
date-format %Y-%m-%d
account1 assets:bank:firsttech:checking
account2 expenses:unknown
description %name
date2 %transacted
comment id:%id
include firsttech-checking.categories.rules
```

**Categorization lives in one per-account file that both import paths
include.** An account may be synced *and* sometimes imported by hand, for
example to backfill history older than SimpleFIN's 90 days. That needs two
rules files, one per CSV shape. The `if` blocks must not be duplicated
between them.

- **Layout.** Each account has a `<account>.categories.rules` file holding
  only `if` blocks. Both the sync rules file and the manual rules file end
  with `include <account>.categories.rules`. Categorization stays per
  account, as Patrick prefers.
- **Refactoring an existing file.** When the wizard finds a manual rules file
  for the same account (its `account1` is the chosen journal account, or its
  name or last-four matches), it asks:

  > "`chase.csv.rules` already categorizes this account (41 rules). Share
  > those rules with sync, so a rule you add on either side works for both?"
  > **[Share them]** · Keep separate · Start the sync rules empty

  **Share them** moves the `if` blocks out of `chase.csv.rules` into
  `chase.categories.rules`, and adds the `include` line to both files.
- **Field names have to line up. Verified on hledger 1.52:**
  - A matcher like `if %description …` only matches when `description` is a
    name in the including file's own `fields` line. An assigned field
    doesn't count. A name the CSV doesn't have isn't an error; the rule just
    never matches.
  - A whole-record matcher (`if grocery`) matches in both shapes.
  - `include` itself works as expected.

  So the sync file's `fields` line **borrows the manual file's names**. If
  the manual file calls the payee column `name`, the sync CSV's payee column
  is named `name` too (as in the example above). We control the sync
  columns, so this is free.
- **Rules that can't be shared** stay in the manual file, and the wizard
  lists them with the reason:
  - a block that tests a field with no sync counterpart (`%trntype`,
    `%checknum`);
  - a whole-record pattern that depends on the row layout (anchored with
    `^`/`$`, or containing a comma).
- **Proof before writing.** The refactor is one write that rolls back
  everything on failure. Before it's committed:
  - Ledgeline runs the manual rules file over its own last saved CSV
    (`csvPath` beside its `.latest`) both before and after the refactor.
    The `hledger print` output must be byte-identical, or the refactor is
    abandoned with the difference shown.
  - The sync side is checked by the step-4 dry run.
  - The dialog shows both files' diffs before **Share them** is final.
- **Remember as a rule** appends to the categories file whenever the
  import's rules file includes one, so a rule learned from a manual import
  also covers sync, and the other way round. Its live check copies both
  files into the temporary area.
- **New capability.** `rules.rs` treats `include` as keep-only today:
  displayed, preserved, and never created by an edit (a security rule; it
  names a path). The refactor adds one **server-side** operation that
  writes `include <sibling>.categories.rules`.
  - The filename is built by the server from the rules file's own stem and
    checked with `discovery::resolve_new`. It's never taken from a client
    string.
  - The editor still can't add or retarget an `include` by hand.
- **Edit Rules** lists a categories file as its own entry, labelled "Used by
  `chase.csv.rules`, `chase.sync.csv.rules`", and edits its `if` blocks as
  usual.
- **Import candidate ranking** skips rules files with no `fields` line. A
  categories file can't read a CSV alone.

### 3.3 Source folder

The user asked for downloads to be saved to a source folder. Each sync writes
the normalised CSV:

```
<journal dir>/imports/sync/<connection-slug>/<account-slug>/2026-11-14T0712.csv
```

- **The folder is a setting:** `syncSourceDir`, default `imports/sync`,
  global or pinned per ledger, like every other `ui_prefs` key.
  - Because it can come from the untrusted `ledgeline.toml`, it must be a
    relative path that resolves inside the journal's folder. An absolute
    path, `..`, or a symlink out of the folder is refused, and the default
    is used with a warning.
  - Ledgeline writes no `.gitignore` (decided).

- **Archive and audit trail.** It's plain CSV, so `hledger import` from a
  terminal gives the same result as Ledgeline.
- **Why one file per fetch, not one growing CSV:** hledger's `.latest`
  date marker would hide late-arriving rows. A pending item that posts dated
  earlier than the last import would be skipped forever. A fresh file per
  fetch has no `.latest`. The id index does the dedup, and the account's
  sync start date (§3.4) protects older history that has no ids.
- **Retention:** keep 90 days of files by default. Older ones are deleted,
  and the setting can be "keep forever".
- Raw JSON responses are never written beside the journal. The last 10 go to
  the app cache dir, with credentials redacted, for "Copy diagnostic report".

### 3.4 Dedup, in four layers

1. **Sync start date per linked account.** It's chosen at link time and
   suggested as the day after the last transaction already posted to that
   journal account. Rows before it are dropped during normalization. This
   protects history imported manually with no id tags, and it keeps the
   existing rule that an id may never add a row.
2. **Id index** (`reimport.rs`, unchanged). Rows are classified as new,
   unchanged, status-changed or conflicting.
3. **Pending: skipped (decided).** Only posted transactions are imported.
   The client never sends `pending=1`. As a second guard, any row with
   `pending: true` or `posted: 0` is dropped. This avoids the pending/posted
   id mismatch entirely, at a cost of 1–3 days' delay. There's no option to
   import pending items.
4. **Possible-duplicate guard (new, `feeds::dupes`).** A new row is treated
   as a possible duplicate when some transaction already on the same journal
   account matches all of these:
   - its id isn't in this fetch;
   - its date is within ±3 days;
   - it has the same amount;
   - its normalised payee matches.

   That catches Capital One's id re-issuing, and an OFX file imported by
   hand for an account that's also synced (FITIDs vs SimpleFIN ids).
   - Possible duplicates are **held, not written**. They wait in the
     account's "Needs a look" list with **Import anyway** and **It's a
     duplicate**.
   - **It's a duplicate** records the new id on the existing transaction as a
     second `id:` tag. `reimport` already treats every `id:` value as a key,
     so the next fetch matches it. *Verify that `build_index` accepts
     several `id` tags on one transaction; if not, extend it.*

### 3.5 Writing: on click always, in the background only when opted in

It works like prices (decided 2026-11-15):

- **Sync now** fetches and writes in one step, the way **Update prices**
  does. The click is the consent. There's no separate stage-then-Write step.
- **Background syncs** (at startup and periodically) happen only when the
  user has turned **Automatic sync** on (`autoSync`, default off). With it
  off, Ledgeline never touches the network or the journal unless asked.

Either way, a sync commits through the same engine function as
`POST /api/import/commit`, called headless the way `ledgeline import` (CLI)
already does:

- **Target:** each linked account names a target journal file at link time.
  It's suggested from the file where that account's latest transactions live.
- **Lock order:** fetch outside the lock, then take the write lock, re-read
  and commit. A conflict with an edit the user is making skips this run
  quietly; the next tick retries, as in plan 41.
- **Transactions are written without `reviewed:`.** They show up in Journal
  → Review (Review Unknowns gets the uncategorized ones), with a badge
  "12 new from sync".
- **What is never auto-written:**
  - conflicting rows;
  - possible duplicates;
  - anything when hledger is missing or the rules file fails.

  Each of these becomes an attention item instead (§6).
- **Balance check:** after each commit, compare the feed's `balance` at
  `balance-date` with the journal balance of the account on that date
  (existing `BalanceField` logic).
  - A mismatch becomes a quiet "Ledger differs from bank by $12.40 as of
    Nov 13" line on the account, with **Show me** linking to running
    balances (plan 36).
  - It's the best detector of missed transactions, so it stays, but it
    never blocks a sync.
  - Some banks report balances with the wrong sign (USAA, BofA). The link
    wizard asks the user to confirm the sign once against their current
    ledger balance, and stores a per-account `balanceSign`.
  - Writing balance assertions automatically is **off**. An option exists
    per account.
- **Transfer matching (plan 35):** after a sync, if the new transactions hit
  clearing accounts, the account row offers "2 possible transfers to pair".
- **First sync of a newly linked account:** this is the one exception to
  writing straight away. The wizard's last step (§4.3, step 4) shows the
  dry run and offers review before **Finish**, because a wrong mapping or
  start date would otherwise write months of bad transactions at once.

### 3.6 Storage: the keychain plus `ledgeline.toml`, no new config files

Decided 2026-11-15: account links go in `ledgeline.toml` beside the journal,
the same file that holds per-ledger theme and settings overrides. There's
no good reason not to, as long as we respect what that file is: something
that travels with a cloned repo, and so is **untrusted**. That rules out
two things: a credential, and a server address. A malicious repo must
never be able to send a stored token to its own host. Everything else in a
link is harmless mapping data.

- **Secrets: OS keychain, via the `keyring` crate.** That covers the macOS
  Keychain, the Windows Credential Manager and the Linux Secret Service.
  - One item per connection, keyed `ledgeline.sync.<connection-uuid>`.
  - The item holds the SimpleFIN access URL, which includes the host. So
    the host comes from the keychain, never from `ledgeline.toml`.
  - Behind a `SecretStore` trait, with an in-memory implementation for tests.
  - On Linux without a Secret Service, the wizard says so and refuses. There
    is no plaintext fallback.
  - The access URL never appears in logs, errors, diagnostics, `cliCommand`
    or the API. Parsing pulls out user and password and sends them as an
    `Authorization` header. Credentials are dropped on any cross-origin
    redirect.
  - The default macOS keychain item doesn't sync through iCloud, so a second
    Mac opening the same ledger shows "Not connected on this computer ·
    Connect…" and needs its own setup token. That's acceptable, and safer.
- **Links, in `ledgeline.toml`:**

  ```toml
  [[sync.connection]]
  id = "4f6c…"                        # uuid → keychain item
  name = "SimpleFIN Bridge"           # display only

  [[sync.account]]
  connection = "4f6c…"
  remoteId = "ACT-12ab…"              # provider account id
  label = "First Tech · Checking ••1234"
  journalAccount = "assets:bank:firsttech:checking"
  rules = "imports/rules/firsttech-checking.csv.rules"
  target = "2026.journal"
  startDate = 2026-11-01
  balanceSign = 1                     # -1 for banks that report it reversed
  enabled = true
  ```

  - The paths `rules` and `target` (and `syncSourceDir`) are resolved with the same
    "inside the journal folder only" checks the importer already uses
    (`discovery::resolve`, `journals::targets_in`). Anything else disables
    that link, with a message saying why.
  - **Writing it.** The wizard, account settings and **Stop syncing** edit
    it with `toml_edit`, preserving comments and order, as the theme pin
    does.
    - Linking an account is an explicit user action, so it may create the
      file. That extends the existing rule, "never auto-created, except an
      explicit pin", to "or an explicit link".
    - Nothing else ever writes `[sync]`.
  - A ledger opened read-only can show links, but can't sync or edit them.
- **Machine state is not config.** Last attempt and success, per-day request
  counts (the quota guard must survive a relaunch), and the last 10 outcomes
  per account go in `sync-state.json` in the app **cache** dir, keyed by
  connection uuid.
  - It's disposable: losing it costs one extra fetch and an empty history.
  - It never goes beside the journal, and it's never something a person
    edits.
- **Settings (`ui_prefs`, pinnable per ledger, so global in `ui-prefs.toml`
  or local in `ledgeline.toml`'s `[ui]` as usual):**

  | Key                      | Default        | Meaning                                                      |
  | ------------------------ | -------------- | ------------------------------------------------------------ |
  | `autoSync`               | `false`        | Sync in the background (§3.5). Off = only on click           |
  | `autoSyncEveryHours`     | `6`            | `3..=24`; a floor protects the 24/day quota                  |
  | `syncSourceDir`          | `imports/sync` | Where fetched CSVs are saved, relative to the journal folder |
  | `syncKeepSourceDays`     | `90`           | `0` = keep forever                                           |

  `autoSync` is forced off under `LEDGELINE_UI_PREFS_IN_BROWSER`, as with
  prices, so tests never touch the network.
  - `syncSourceDir` is an ordinary `ui_prefs` key, so its local override
    lives in `[ui]` like any other. `[sync]` holds only connections and
    links.

### 3.7 Scheduler (`auto_sync.rs`, a copy of the `auto_prices.rs` shape)

`plan_sync(now, links, connections, history, prefs, backoff) -> SyncPlan` is
a pure function. It returns one of these per connection:

```
Off | ReadOnly | NoLinks | NotDue{next} | BackingOff{until} | NeedsUser{reason} | Fetch{window, accounts}
```

**"Most recent end of day":**

- A connection is due if its last successful sync is before today's 07:00
  local time. MX refreshes overnight, and before 07:00 the latest end of day
  is yesterday's.
- It's also due if the last success is more than `autoSyncEveryHours` old.
- The app runs a first check 45 s after launch, plus a random 0–10 min
  jitter.
- Then it wakes every 5 min by wall clock, which survives sleep, as plan 41
  does.

**Window:**

- `start = max(sync start date, last success − 5 days)`, `end = tomorrow`.
- The first sync takes `start = sync start date`. If that's more than 90 days
  back, the request is split into 90-day chunks, oldest first.
  - Each chunk costs one request, and the chunks count against the quota.
  - The wizard warns when the history asked for needs more than 4 chunks.
  - Data older than the institution keeps is reported plainly: "First
    National only provides history back to Aug 20".

**Quota guard** (per connection, persisted in `sync-state.json`, because a
relaunch must not reset it):

- Auto syncs stop at 6 requests per day.
- **Sync now** works until 16 per day.
  - Past 12, the button says "12 syncs today. SimpleFIN allows about 24
    before it disables access."
  - At 16 it refuses until midnight UTC, with that reason.
- Any quota warning from the server sets an auto-sync backoff until the
  next UTC day.

**Backoff** on network / 5xx errors: 15 min, doubling, capped at 6 h. It
lives in memory.

**Sync now:** skips `NotDue`, and still respects the hard quota and
`NeedsUser`. It can target one connection, or one account (using the
`account=` parameter, which has its own quota on the Bridge).

**Status:** `GET /api/sync/status` is polled like prices status. It returns
per connection and per linked account:

```
lastAttempt, lastSuccess, lastOutcome, added, held,
balanceDiff, problems[], nextAuto, requestsToday
```

---

## 4. GUI

### 4.1 Where it lives

Imports gains a first tab: **Accounts**. That makes three tabs:
Accounts · New Transactions · Edit Rules. Keys <kbd>1</kbd>–<kbd>3</kbd>;
existing deep links are kept.

There are also two global hooks:

- **Header:** a sync status dot. It's hidden when nothing is linked. It shows
  idle, a spinner while syncing, amber when something needs attention, and
  red for a broken connection. Clicking it opens Imports → Accounts.
- **Problems drawer:** broken connections and held items appear in the
  existing `ProblemsBadge` / `/api/diagnostics` list, each with its fix
  button.

### 4.2 Imports → Accounts (desktop width)

```
┌ Accounts ─────────────────────────────────────────────────────────────── [ Sync now ] ┐
│ Auto-sync on · last synced 7:12 AM · next around 1:15 PM          ⚙ Sync settings     │
│                                                                                       │
│ SimpleFIN Bridge  ● Connected                     who sees your data ⓘ    ⋯           │
│ ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│ │ First Tech · Checking ••1234   → assets:bank:firsttech:checking                 │   │
│ │   Synced 7:12 AM · 4 new · 2 to review · balance matches ✓            [Review]  │   │
│ │ Chase · Sapphire ••9921        → liabilities:cc:chase                           │   │
│ │   ⚠ Chase needs you to sign in again at SimpleFIN.  Data stops at Nov 11.       │   │
│ │                                            [Fix at SimpleFIN ↗]  [I've fixed it] │   │
│ │ Ally · Savings ••0077          → not linked                         [Link…]     │   │
│ └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                       │
│ Manual downloads                                                                      │
│ ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│ │ Vanguard Brokerage             → assets:invest:vanguard                         │   │
│ │   Last imported Oct 2 (43 days ago) · last transaction Sep 30                   │   │
│ │                                  [Open download page ↗]  [Import a file…]       │   │
│ └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                                                                       │
│ [ + Connect a bank ]                                                                  │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

- **Manual downloads** lists the journal's asset and liability accounts that
  aren't linked. They're classified by declared account type (memory
  `account-type-not-name`).
  - Each row shows the last transaction date, and the last import date when
    an import's CSV is known.
  - The per-account "download page" URL is an optional field the user can
    add.
  - **Import a file…** jumps to New Transactions with the account preset.
  - A row turns amber after 35 days with no new transactions, a nudge that
    can be switched off per account.
- **⋯ on a connection:**
  - Rename
  - Sync this connection now
  - Re-check accounts (finds new accounts linked at the Bridge)
  - Replace token…
  - Copy diagnostic report
  - Disconnect…
- **Click an account row** to open a drawer with:
  - its mapping: account, target file, and rules file, with **Change…**
    (the same choices as the wizard) and **Edit** (opens Edit Rules). The
    categories file is shown under it with its other users;
  - sync start date, balance sign;
  - the last 10 sync results (time, added, held, outcome);
  - **Needs a look:** possible duplicates and conflicts, each with its two
    buttons;
  - **Stop syncing** (keeps history, removes the link).
- **Narrow / phone widths** stack the same rows, as plan 39's container
  queries already do. It's desktop-only for now (§1.4) but laid out to
  survive a narrow window.

### 4.3 Connect-a-bank wizard

The wizard is a modal with a step rail. Every step has **Back**, and nothing
is written until the last step.

#### Step 1: Choose how to connect

```
How should Ledgeline get your transactions?

( ) Through SimpleFIN Bridge                        Third party · $15/year
    Works with most US banks. SimpleFIN and its data provider MX can see
    your transactions; Ledgeline never sees your bank password.
    You'll set it up on SimpleFIN's website, then paste a token here.

( ) I'll download statements myself                 Nobody else involved
    Always works. Ledgeline reminds you when an account is getting stale.

Why the difference? ⓘ  (expands the "who sees your data" table, §2)
```

- The Accounts tab's **+ Connect a bank** opens this step. Choosing
  "download myself" closes the wizard and highlights that account's row
  under Manual downloads.
- A bank's own direct option would be added here as another choice, if we
  ever build one.

#### Step 2: SimpleFIN setup (guided, outside the app)

```
Set up SimpleFIN Bridge                                   About 5 minutes

 1. Create a SimpleFIN Bridge account and subscribe.      [Open SimpleFIN ↗]
 2. On SimpleFIN, choose "Connect an institution" and sign in to your bank.
    Do this for each bank you want. (You can add more later.)
 3. On SimpleFIN, choose "New app connection", name it "Ledgeline",
    and copy the setup token it shows.
 4. Paste the token here:
    ┌──────────────────────────────────────────────────────────┐
    │ aHR0cHM6Ly9iZXRhLWJyaWRnZS5zaW1wbGVmaW4ub3Jn...          │
    └──────────────────────────────────────────────────────────┘
    ✓ Token from beta-bridge.simplefin.org                [Connect]

 A setup token works once. Ledgeline exchanges it for a private access key
 and keeps that in your keychain. You can revoke it on SimpleFIN any time.
```

- **Live validation:** the field base64-decodes the token and shows its
  host. An unknown host is allowed (other SimpleFIN servers exist), with "Not
  SimpleFIN Bridge. Only continue if you trust beta-bridge-example.com."
  Plain `http` is refused.
- **Paste detection:** when the window regains focus and the clipboard holds
  something that decodes to a `…/claim/…` URL, the field offers "Paste
  token from clipboard". It never reads the clipboard silently.
- **Claim errors, worded for people:**

  | Cause                       | Message and action                                                                                                                                                                                                                    |
  | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
  | 403 from the claim endpoint | "This token was already used. If you didn't use it in another app, someone else may have, so disable it on SimpleFIN. Then create a new one and paste it here." **[Open SimpleFIN ↗]**                                               |
  | 403 at the edge (Cloudflare) | "SimpleFIN's firewall refused the connection. Try again in a few minutes."                                                                                                                                                           |
  | Network error               | "Couldn't reach beta-bridge.simplefin.org. Check your connection." **[Try again]**                                                                                                                                                  |

- **After the claim succeeds:** store in the keychain, then run a
  balances-only fetch. Zero accounts → "SimpleFIN has no banks connected yet.
  Connect one on SimpleFIN, then press Re-check." **[Re-check]**

#### Step 3: Link accounts

```
SimpleFIN found 5 accounts. Choose where each one goes.

First Tech · Checking ••1234     $4,210.55
  Journal account [ assets:bank:firsttech:checking ▾ ]   ledger: $4,198.15 ✓ close
  Start syncing   [ Nov 1, 2026 ]  (your journal has this account through Oct 31)
  Write into      [ 2026.journal ▾ ]
  Rules file      [ New: firsttech-checking.sync.csv.rules ▾ ]
Chase · Sapphire ••9921          −$1,032.18
  Journal account [ liabilities:cc:chase ▾ ]             ledger: −$1,032.18 ✓ matches
  Rules file      [ New: chase.sync.csv.rules ▾ ]
  ⓘ chase.csv.rules already categorizes this account (41 rules).
    Share them with sync?   [Share them]  Keep separate  Start empty
  ...
Ally · Savings ••0077            $20,000.00
  Journal account [ Don't sync ▾ ]
  ( ) New account: [ assets:bank:ally:savings ]  opening balance $20,000.00 on [Nov 13]
```

- **Suggestions:**
  - by last-four digits and institution name against account names and
    existing rules files' `account1`;
  - by matching balance;
  - reusing `qb_import::suggest_accounts` scoring.
- **Balance sign check:** when the bank's balance is the negative of the
  ledger's, a one-line question appears: "Chase reports this as +1,032.18.
  Your ledger has −1,032.18. Treat Chase's sign as reversed?"
- **New account:** writes an `account` declaration with its type, plus an
  opening-balance transaction. This is plan 39's OFX seeding idea.
- **Rules file:** chosen per account (§3.2). The menu offers "New: …"
  (default) or any existing file that fits the sync shape. **Share them**
  opens the refactor dialog: what moves, what stays and why, and both
  files' diffs.
- **Start date:** defaults to the day after the account's last journal
  transaction. For a new account it's the earliest the feed offers, with the
  90-day chunk note.

#### Step 4: First sync preview

- Runs a dry run per linked account.
- Shows counts per account: "38 new · 0 already in your journal · 2 possible
  duplicates held · 6 uncategorized".
- **Review now** opens the existing card review over the combined import.
- **Finish** writes everything:
  - the links (`[sync]` in `ledgeline.toml`);
  - the rules files;
  - the first import.
- Then a toggle: **Keep these up to date automatically** (sets `autoSync`),
  with the cadence explained in one line.

### 4.4 Settings → Sync

This is a new panel modelled on `PricesSettingsPanel`:

- auto-sync toggle, interval, source folder, keep-source days;
- "Use for this ledger only" pins;
- a list of connections, with **Disconnect**.

**Disconnect** wording:

> "Removes the access key from this Mac. Your transactions stay. To stop
> SimpleFIN itself sharing data, also disable the 'Ledgeline' app on
> SimpleFIN." **[Open SimpleFIN ↗]**

### 4.5 Feedback during and after a sync

- **While Sync now runs:** the button spins. Rows update as each connection
  finishes.
- **Toast on finish:** "Synced 3 accounts · 9 new transactions ·
  **Review 4**".
- **Auto-sync stays quiet** on success. The Accounts tab's status line and
  the header dot change, and nothing pops up.
- **Auto-sync failures don't toast.** They set the dot and a Problems item,
  so a laptop that's offline for a weekend doesn't nag.

---

## 5. Data flow, end to end

```
 Scheduler tick / Sync now
        │ plan_sync (pure)
        ▼
 FeedProvider.fetch ── network, outside any lock, User-Agent "Ledgeline/x.y"
        │ FeedBatch (normalised, problems[])
        ▼
 per linked account: filter by start date, drop pending → Tabular → id prefixing
        │ write imports/sync/<conn>/<acct>/<ts>.csv
        ▼
 dry run (existing import engine, headless): rules → hledger → proposal
        │ reimport classify → dupes guard → split: writable | held
        ▼
 write lock → commit writable rows into target file (existing commit path)
        │ balance compare → status record → diagnostics
        ▼
 SPA notices generation bump (existing poll) → Review badge, Accounts tab
```

---

## 6. Errors and how to fix them

Every problem has the same five fields:

- **who** (your bank / SimpleFIN / your setup in Ledgeline / network);
- a **plain sentence**;
- **since when** ("Data stops at Nov 11");
- **one primary action**;
- a **details** disclosure with the raw code and the server's message
  (sanitized).

| Trigger                                   | Scope       | Shown as                                                                      | Primary action                                               |
| ----------------------------------------- | ----------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `con.auth` / v1 "may need attention"      | institution | "Chase needs you to sign in again at SimpleFIN."                              | **Fix at SimpleFIN ↗** (org URL), then **I've fixed it** (re-sync that account) |
| `act.failed` / `act.missingdata`          | account     | "SimpleFIN couldn't fetch this account today. Usually temporary."             | none; becomes red after 3 days → **Fix at SimpleFIN ↗**     |
| HTTP 403 on `/accounts`                   | connection  | "Ledgeline's access to SimpleFIN was turned off or revoked."                  | **Reconnect…** (paste a new token; links are kept)           |
| HTTP 402                                  | connection  | "Your SimpleFIN subscription needs renewing."                                 | **Open SimpleFIN ↗**                                         |
| quota warning in `errlist`                | connection  | "Close to SimpleFIN's daily limit. Auto-sync paused until tomorrow."          | none                                                         |
| network / 5xx / timeout                   | connection  | amber only after 24 h of failures: "Can't reach SimpleFIN since…"             | **Try again**                                                |
| no new data for N days, no errors         | account     | "No new transactions from SimpleFIN since Nov 2. Normal for quiet accounts." | **Fix at SimpleFIN ↗** (often a silent bank break)           |
| hledger missing / too old                 | ledger      | existing hledger banner text                                                  | **Set hledger path**                                         |
| rules file error (hledger stderr)         | account     | "The rules for this account stopped working." + stderr                        | **Edit rules**                                               |
| conflicting / possible duplicate rows     | account     | "2 transactions need a look before they're added."                            | **Review**                                                   |
| balance mismatch                          | account     | "Ledger differs from bank by $12.40 as of Nov 13."                            | **Show me** (running balance)                                |
| target file missing / read-only           | account     | "Can't write to 2025.journal."                                                | **Choose another file**                                      |
| link refers to unknown connection         | account     | "Not connected on this computer."                                             | **Connect…**                                                 |

- **Copy diagnostic report** lists, with the access URL redacted (`redact.rs`):
  - connection kind and host;
  - the last 10 outcomes with codes;
  - the last response's `errlist`;
  - app version.

  It's for support or a GitHub issue.
- **Docs:** `docs/imports/bank-sync.md` gets one anchored section per row
  above. Every in-app details disclosure links to its anchor.

---

## 7. Security checklist

- HTTPS only, with certificates verified (rustls). This is a SimpleFIN spec
  requirement. A test-only escape allows `http://127.0.0.1` for the fake
  server, behind `cfg(any(test, feature = "test-fakes"))`.
- The access URL only ever exists in the keychain, in memory, and in the
  `Authorization` header.
  - `redact.rs` scrubs `user:pass@`.
  - No `Debug` derive on `Secret`.
- Credentials are dropped on a cross-origin redirect.
- Custom currency URLs (`currency: "https://…"`) are fetched only from the
  connection's own host. Results are cached, and `name` / `abbr` are
  sanitized. The plain fallback is the commodity symbol `"?"` plus a warning.
- Every server string (`msg`, names, descriptions) is shown as text, never as
  HTML. Descriptions go into hledger via CSV quoting, and `%` is escaped the
  same way `ruleBuilder` handles it.
- A one-time-use claim 403 tells the user about the possible compromise, as
  the spec requires.
- Sync endpoints need the bearer token like every other `/api/*`, and are
  only enabled when `editing_enabled()`.

---

## 8. Testing

| Layer                  | What                                                                                                                                                                                                                                                                                                                                                   | Where / how                                                                                     |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| Parsing                | v1 and v2 `/accounts` bodies: demo recording, plus synthetic pending, custom currency, `errlist` codes, `x-api-message`, unknown extra fields, numeric strings without two decimals, `posted: 0`                                                                                                                                                         | `ledgeline-core` unit tests; fixtures in `fixtures/feeds/simplefin/`                            |
| Token                  | base64 decode, host extraction, http refused, unknown host flagged, junk input                                                                                                                                                                                                                                                                          | unit + proptest (no panic on any input)                                                         |
| Normalization          | id prefixing, tag safety, start-date filter, pending policy, zero-amount drop, date/timezone rule, sign handling                                                                                                                                                                                                                                         | unit; golden CSV per fixture                                                                    |
| Rules template         | the generated feed rules file is accepted by real hledger                                                                                                                                                                                                                                                                                              | add to `just rules-check`                                                                       |
| Rules sharing          | categories refactor: blocks moved vs kept (field with no sync counterpart, anchored or comma whole-record pattern); sync `fields` borrows manual names; before/after `hledger print` byte-identical on fixture CSVs, abort + rollback when not; `include` path built server-side only; candidate ranking skips field-less files; Remember-as-rule lands in the categories file | `ledgeline-core` rules tests + `fixtures/import/shared-categories/`; hledger checks via `just rules-check` |
| Dedup                  | id-churn re-issue → held; manual OFX + sync on same account → held; pending rows never written; same id identical → unchanged; status flip; legacy untagged history before the start date never duplicated                                                                                                             | core unit + `fixtures/import/reimport/` style folders                                           |
| Planner                | `plan_sync` with fixed clocks: 06:59 vs 07:01, interval, jitter bounds, sleep/wake gap, quota counters across a relaunch, backoff, `NeedsUser` short-circuit, 90-day chunking                                                                                                                                                                             | pure unit tests, like `auto_prices.rs:827+`                                                     |
| Error mapping          | every row of §6's table: input → problem record → wire shape                                                                                                                                                                                                                                                                                            | unit                                                                                            |
| Fake server            | an in-process axum "SimpleFIN" (axum is already a dependency; no new mock crate) that scripts responses: claim 200/403, Cloudflare-style 403, 402, 403, `con.auth`, quota warning, 90-day cap, slow and 5xx. Tests the real `reqwest` client: headers, auth, User-Agent, redirect credential drop                                                       | `crates/ledgeline-server/tests/sync_endpoints.rs`                                               |
| Engine end to end      | fixture ledger copy + `FakeFeed` → `run_sync_once(now)`, checking exact journal bytes, source files, `ledgeline.toml` `[sync]`, status JSON. **Idempotence:** run twice → second adds nothing. Conflict mid-run → skipped, then retried. Read-only journal → nothing                                                                                       | same file; pattern from `prices_endpoints.rs`                                                   |
| Secrets                | `SecretStore` in-memory fake in all tests; real keychain round-trip as an `#[ignore]` test run by hand on macOS                                                                                                                                                                                                                                        | server tests                                                                                    |
| Front end              | wizard step models (pure), status line wording, problem → action mapping, account suggestion ranking                                                                                                                                                                                                                                                    | vitest `unit` + `components`                                                                    |
| Live API shape         | `just test-integration` against the server with `LEDGELINE_FEED_FAKE=fixtures/feeds/simplefin/scenario-*.json`: a launch flag that swaps in the fake provider and fails closed in release builds                                                                                                                                                       | `*.integration.test.ts`                                                                         |
| e2e                    | wizard happy path, broken-connection fix path, Sync now with held items. Uses the fake provider, never the network; produces docs screenshots. Run by Patrick or the main session, not by worktree agents                                                                                                                                              | `web/e2e/sync.e2e.ts`                                                                           |
| Live smoke (opt-in)    | `just test-simplefin-demo`: claim `DEMO`, list, fetch 30 days, parse. Never in CI; one run is 2–3 requests                                                                                                                                                                                                                                              | manual                                                                                          |
| Real-bank QA           | Patrick, with his own Bridge account, on a **copy** of a scratch ledger. Checklist: link 2+ banks, first sync, a day later, a purchase going from pending to posted over 3 days (it must appear once, when it posts), deliberately break a connection on the Bridge, revoke the token, quota warning. Claude never reads the real data (memory `no-private-financial-data`); Patrick reports outcomes and redacted diagnostics | manual checklist in this plan's appendix (Phase 3 exit)                                         |

---

## 9. Phases

Each phase is shippable, and each ends with docs and screenshots.

### Phase 1: Core and client (no UI)

- `feeds` module, the SimpleFIN parser for v1 and v2, normalization, the
  feed rules template, `dupes`.
- `SecretStore` (keyring + memory), the `ledgeline.toml` `[sync]` model,
  `sync-state.json`.
- The SimpleFIN `FeedProvider` (claim, accounts, error mapping).
- Shared categories: the refactor operation and its byte-identical check,
  the server-built `include` write, ranking that skips field-less files.
- Fake server and parsing tests.
- CLI for early dogfooding:
  - `ledgeline sync connect <token>`
  - `ledgeline sync status`
  - `ledgeline sync now`
- **Exit:** Patrick links a scratch ledger from the CLI with his real Bridge
  account. Two days of syncs produce no duplicates.

### Phase 2: Engine

- `auto_sync.rs`: the planner, quota and backoff.
- Headless import commit, held items, balance compare.
- `/api/sync/*`:
  - `status`
  - `connect` (claim)
  - `accounts` (list)
  - `link`
  - `unlink`
  - `now`
  - `resolve` (duplicate / import anyway)
- Diagnostics integration. The `FeedProvider` fake flag.
- **Exit:** engine end-to-end and idempotence tests are green;
  `just test-integration` passes.

### Phase 3: UI

- Accounts tab, the wizard (SimpleFIN + manual), the account drawer,
  Settings → Sync, the header dot, Problems items.
- Docs: `docs/imports/bank-sync.md` with the tradeoff table, the setup
  walkthrough and a troubleshooting section per error; e2e and screenshots.
- **Exit:** the real-bank QA checklist passes.

### Phase 4 (later, not planned in detail)

- Auto balance assertions.
- iOS: fetch-only, or sync on the desktop and read the journal through
  iCloud, which already works.
- **Plaid with the user's own keys (§1.2b).** Start with a ½-day spike:
  - create a Trial team;
  - run Plaid Link inside wry's webview, including an OAuth bank's popup;
  - check Schwab and Fidelity coverage;
  - confirm what the 10-Item lifetime cap means in practice.

  If it all works, build:
  - the provider (cursor sync, "removed" handling);
  - a wizard path, "Advanced: use your own Plaid account", with the
    developer-dashboard steps spelled out;
  - a Linked-logins counter "7 of 10 used" that warns before re-linking.
- A specific bank's own direct API, if one is worth it. OFX Direct Connect
  stays out: see §1.2.

---

## 10. Decisions (Patrick, 2026-11-15)

| #   | Question                         | Decision                                                                                                                                                                                                                         |
| --- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | When does a sync write?          | Like prices. **Sync now** fetches and writes on click. Background syncs (startup and periodic) happen only with the opt-in `autoSync` setting on. The first sync of a newly linked account is previewed in the wizard. (§3.5)  |
| 2   | Pending transactions             | Skipped. Only posted ones are imported, and there's no option to change that. (§3.4)                                                                                                                                             |
| 3   | OFX Direct Connect               | Hidden and not planned. SimpleFIN direct doesn't exist and OFX is disappearing. A specific bank's own API may be added later. (§1.2)                                                                                              |
| 4   | Where links live                 | `[sync]` in `ledgeline.toml`, so there's no new config file. Secrets go in the keychain; machine state goes in a disposable cache file. (§3.6)                                                                                    |
| 5   | Source folder                    | `imports/sync` by default, overridable via the `syncSourceDir` setting (global or per ledger). No `.gitignore`. (§3.3)                                                                                                           |
| 6   | Dates                            | Posted date as the transaction date, and `transacted_at` as hledger's secondary date when it differs. (§3.2)                                                                                                                      |
| 7   | Rules file per account           | Picked at link time; changeable later from the account drawer. (§3.2)                                                                                                                                                            |
| 8   | Shared categorization            | Per-account `<account>.categories.rules`, included by both the sync and manual rules files. When a manual file exists, offer to refactor its `if` blocks into it, with no duplication. (§3.2)                                     |
| 9   | MX / Stripe directly?            | Researched (§1.2b). MX has no individual path. Stripe is aimed at businesses, its personal use is unclear, and its data is thin. Plaid's new free Trial plan (own keys, no server) is the realistic direct option, slotted as Phase 4. *Pending Patrick's agreement.* |

## 11. Still open

- **Timezone of SimpleFIN epochs.** Check on Patrick's real data in Phase 1
  (§3.2).
- **Several `id:` tags on one transaction.** **It's a duplicate** relies on
  this. Check that `reimport::build_index` indexes every `id:` value.
- **Opening-balance transaction for a newly created account (§4.3,
  step 3).** Write it into the target file or the file holding the
  journal's other opening balances?

## Appendix: research sources

- SimpleFIN protocol v2: https://www.simplefin.org/protocol.html (checklist, errors, custom currencies)
- Bridge developer guide (rate limits, 90 days, demo): https://beta-bridge.simplefin.org/info/developers
- Bridge security, privacy, terms and mission: https://beta-bridge.simplefin.org/info/security, https://beta-bridge.simplefin.org/info/privacy, https://beta-bridge.simplefin.org/info/terms, https://beta-bridge.simplefin.org/info/mission
- Bridge issues: #5 (no self-host), #7 (credentials in URLs), #8 (MX UUIDs), #19 (day-granular filtering), #22/#31/#32 (data quality), #26 (`/info` version), #27 (Cloudflare 403), #28 (Capital One id re-issue): https://github.com/simplefin/bridge-issues
- v1→v2: https://github.com/simplefin/simplefin.github.com/discussions/33; `mcc` PR #40
- Ecosystem (no bank servers): https://www.simplefin.org/ecosystem.html; Synci https://synci.io/destinations/simplefin; Redbark https://www.redbark.co
- Actual: `packages/sync-server/src/app-simplefin/app-simplefin.js`; pending double-count issue actualbudget/actual#8932
- Firefly III importer: `config/simplefin.php`, `TransactionTransformer.php` (89-day chunks, `transacted_at`)
- Sure pending matching: we-promise/sure#3306
- OFX: ofxtools client docs and issues #93, #140, #142, #158, #185, #193, #200; GnuCash wiki "OFX Direct Connect Bank Settings"; ofxhome dump https://github.com/ddoherty03/ofxhome_dump
- Plaid: quickstart, OAuth, Hosted Link, `/transactions/sync` docs at https://plaid.com/docs/
- Verified live on 2026-11-14:
  - `claim/DEMO` can be claimed repeatedly;
  - the 90-day cap returns `gen.api`;
  - a missing User-Agent gets the Cloudflare 403;
  - Fidelity's, Chase's and Schwab's OFX hosts don't resolve.
- Direct-to-aggregator research (2026-11-15):
  - Plaid changelog: Trial plans 2026-04-16; OAuth for Trial 2026-05-20. https://plaid.com/docs/changelog/
  - Plaid billing: https://plaid.com/docs/account/billing/
  - Stripe pricing and Financial Connections: https://stripe.com/pricing, https://docs.stripe.com/financial-connections/transactions
  - Teller: https://teller.io/docs/guides/connect
  - MX signup and connect flow: https://dashboard.mx.com/sign_up, https://docs.mx.com/products/connectivity/
  - Enable Banking: https://enablebanking.com/docs/api/linked-accounts/
  - Akahu: https://developers.akahu.nz/docs/personal-apps
  - GoCardless closure (signups closed since July 2025): https://actualbudget.org/docs/advanced/bank-sync/gocardless/
  - Not confirmed: Plaid's paid rates (third-party estimates only); Plaid's Schwab coverage; whether Stripe allows personal use; Teller's terms.
