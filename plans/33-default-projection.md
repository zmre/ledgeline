# 33 — A default projection

Patrick puts a lot of work into one scenario file and wants the Projections tab
to open on it. Today he has to pick it from the scenario picker every time he
launches the app. This plan adds a **default scenario file per journal**. The
default is a header comment, `; default-scenario: true`, in the scenario file itself, so
everything in the journal folder stays plain hledger. It is set with a star in
the scenario file bar, and shown and cleared in Settings. With no default set,
the tab behaves exactly as it does today.

Three PRs:

- **Phase 1:** the marker and the wire. This is engine work only, with no
  visible change.
- **Phase 2:** the Projections tab. It opens the default and has the star in
  the file bar.
- **Phase 3:** a Settings → Projections tab, plus docs.

## The ask (Patrick, 2026-11-09)

> Default projection: once I've put a lot of work into a projection, I want to
> see it when I go to the projections tab without having to load it as a
> separate step. I like our default as-is, but maybe we can have a setting that
> lets us auto-load a default file and use today's behavior if one isn't
> specified.

Decisions Patrick made before this plan was written are marked **(P)** below.

## What is true today

### Why the tab forgets what was open

- **The tab already tries to reopen the last file.** `start()`
  (`routes/projections/+page.svelte:114-122`) reads
  `settings.lastProjectionId`, opens that file if it is set, and otherwise
  seeds from the journal. `fileStore.adopted` writes the id on every load and
  save (`fileStore.svelte.ts:219-224`), and `forget` clears it (:234-239).
- **`lastProjectionId` is kept in localStorage**
  (`stores/settings.svelte.ts:68-82`). In the desktop app, localStorage starts
  empty on every launch, because the GUI binds a new port each time and WebKit
  keys storage by origin. TODO.md, "Known issues", records the cause and the
  evidence. That is why Patrick has to load his projection by hand. This plan
  does not depend on fixing it, because the default lives in the scenario
  file.
- **Coming back to the tab reloads the file from disk.**
  - `onServerReady`'s `attempted` latch belongs to each component instance
    (`serverWatch.svelte.ts:31-42`). Leaving the tab and coming back mounts a
    new page component, so `start()` runs again.
  - `start()` then reopens `lastProjectionId` with `openFile` → `adopt`
    (`+page.svelte:117`, :137). `adopt` resets `dirty`
    (`scenarioStore.svelte.ts:203-209`).
  - As read, this means **unsaved edits to a loaded file are thrown away when
    you switch tabs and come back.** By contrast, `ensureSeeded` refuses to
    replace dirty work (:224-233).
  - Phase 2 first pins this with a failing test. Patrick's decision 2 requires
    fixing it in any case.

### The scenario file header

- **What it looks like.** A file Ledgeline saved starts with this block
  (`serialize.rs:1461-1488` `render_header`; `BANNER` is at :139):

  ```
  ; Ledgeline projection
  ; projection: Series A with a hiring ramp
  ; created: 2026-09-18
  ; updated: 2026-11-09
  ; cash: assets:checking
  ```

  - Each line is `; <key>: <value>`. The keys are lower-case single words,
    written by `header_line` (:3001-3013), which refuses a comma because it
    would split a tag.
  - The header is **rewritten in full on every save**, from fields of
    `Scenario` (`projections.rs:574-585`; read back at `serialize.rs:476-484`).
    `created:` is carried through from the load, and `updated:` is set by the
    engine.
- **Two places recognise header keys:**
  - `header_span` / `is_header_comment` (`serialize.rs:3116-3150`): the span
    the writer replaces. It **stops at the first comment line whose key it
    does not recognise.** An unknown key placed mid-header would cut the span
    short, and the next save would then write a second header.
  - `discovery::parse_header` (`discovery.rs:570-605`): the listing's read.
    Its first match wins, and keys are compared ASCII-case-insensitively.
- **hledger ignores these lines.** They are top-level comment lines, not
  attached to any transaction or posting, so they are neither tags nor
  data. Scenario files already rely on this: `; projection:` and `; cash:`
  are the precedent.
- **The listing already reads headers**, so reading one more key costs
  nothing.
  - `read_header` (`discovery.rs:552`) reads at most `MAX_HEADER_BYTES` (4 KiB)
    of **`projection-*.journal` files only** (:513-514, plan 22 amendment 32).
    "Other journals", which can be a main ledger tens of megabytes long, are
    never read for a header.
  - `Discovery::without_headers_in` skips even that, and `document` and `save`
    use it.
  - Files are sorted projection files first, then by id (`discovery.rs:371`).
- **The write path.**
  - `PUT /api/projections/{*id}` (`projections_api.rs:2857-2940`) runs under
    `state.import_writes()` (:2893).
  - It checks the client's `revision` against the raw-bytes fingerprint (409
    `stale` otherwise). It then writes through `Vault::replace` pinned to
    `Revision {fingerprint, identity}`, which also refuses with `Changed` if
    the file moved between the read and the rename.
  - A no-op writes nothing.
  - `create_scenario` (:2574) and `update_scenario` (:2671) render the header
    from the decoded scenario.
- **Files that cannot be written.** A file that the main journal includes is
  `writable: false` (decision 5, `writable_path`). When no editor is bound,
  every write answers 501 `editing_disabled()` (`error.rs:95-100`;
  `projections_api.rs:2865`).
- **The disk poll** (`+page.svelte:162-189`, `fileStore.checkDisk` :145-161)
  re-reads the open file every 5 s. If the revision differs from the one held,
  it either adopts the new file (when nothing is dirty) or shows "changed on
  disk" (when something is dirty). A check that overlaps a load or save is
  `skipped` (`superseded`, :151).
- **Older clients and wire compatibility.**
  - `decodeProjectionIndex` and `decodeScenarioFile` ignore unknown keys
    (`api/nativeDecode.ts:2844-2900`).
  - `ScenarioIn` is `deny_unknown_fields`, so a new **input** field would
    break an older client's save. A new **output** field does not.
  - The index is pinned only by the inline `PROJECTION_INDEX`
    (`nativeDecode.test.ts:2112`) and its `DECODERS` triple (:2593).
    `projections-asset` (a `GET` of one file) is a native golden.

### The UI pieces this touches

- `ScenarioFileBar.svelte`:
  - the select is at :76-102, and "Seeded from your journal" is the `""`
    option at :87;
  - Save and Save as are at :105-125;
  - the inline discard confirm is at :128-137.
- The Settings route has four tabs from `TAB_ORDER` (`settings/params.ts:5-8`),
  with keys 1–4 (`SettingsTabs.svelte:15`).
- e2e (`web/e2e/projections.e2e.ts:6-21`) is read-only by design (the
  committed `fixtures/sample.journal` on :5099). Specs that write use a
  scratch engine copied into `target/e2e-*` (`playwright.config.ts:41`, :54,
  :67). Ports 5096–5099 are taken.
- The docs are `docs/projections/index.md` ("Where the files live", :268-330;
  the header is described in `file-format.md`) and `docs/settings/index.md`
  (":12 Settings has four tabs").

## Decisions

### Behaviour

**D1. One default per journal, stored on the server. (P)** It is set from the
scenario file bar and shown and cleared in Settings.

**D2. Precedence. (P)**
- On the first mount after the page loads, the default **always** opens. A
  page load is a launch, a reload (⌘R) or a File→Open.
- Within a session, coming back to the tab keeps whatever was open, including
  unsaved edits.
- With no default, the tab behaves exactly as today: `lastProjectionId`, else
  a seed.

**D3. "Within a session" means "a scenario has been adopted since the page
loaded".** `start()` returns straight away when `scenarioStore.adopted` is
true.
- This applies to a reconnect (nonce bump) as well as a tab switch.
- It also fixes the unsaved-edits loss above.

**D4. The default is stored in the scenario file, with no Ledgeline-only file
in the folder. (P)** The marker is a header line:

```
; Ledgeline projection
; projection: Series A with a hiring ramp
; created: 2026-09-18
; updated: 2026-11-09
; cash: assets:checking
; default-scenario: true
```

- **Spelling: `; default-scenario: true` (P).** It follows the existing
  `; <key>: <value>` shape, so both parsers split it the same way. The key is
  `default-scenario` rather than a bare `default` so it cannot be read as the
  tab's "Default cash account", which is `; cash:`. A boolean value rather than
  a bare `; default-scenario` keeps it the same shape as every other header
  line.
  - The read is ASCII-case-insensitive on the key and accepts only `true`
    (case-insensitive) as the value. Anything else (`false`, `yes`, empty)
    means "not default". This keeps the read strict and leaves nothing
    ambiguous.
  - hledger ignores it, as it does the other header lines (verified by a test,
    Phase 1).
- **It is written last in the header**, after `cash:`. An older Ledgeline,
  which does not know the key, then sees the header end just before it and
  keeps the line as an ordinary comment rather than losing it or cutting the
  header short. See Risks.
- **Clearing removes the line.** It is not rewritten as `; default-scenario: false`.
  "No marker" is already "not default", and a `false` line would leave a
  meaningless line in every file that was ever a default.

**D5. Only Ledgeline's own scenario files can be the default: a
`projection-*.journal` that the tab can write. (P)**
- **Other journals:** the listing never reads their header (plan 22 amendment
  32, because they can be enormous). Writing a Ledgeline marker into a budget
  file or last year's ledger is also out of character for a tool that
  "never writes to a plan of record".
- **Read-only files** (included by the main journal): the marker cannot be
  written.
- In both cases the star is disabled with a reason, and Settings does not
  offer the file. "Save as" turns either into a projection file that can be
  starred.

**D6. If several files carry the marker, the first one in listing order
wins. The warning shows on the Projections tab and in Settings. (P)**
- Listing order is projection files first, then by id. Only projection files
  are read, so in practice this means the **first by id.** The rule is
  deterministic and visible, and it is the order the picker shows.
- **The conflict is surfaced, not repaired silently.** The index adds a
  warning: "2 scenario files are marked as the default (projection-a.journal,
  projection-b.journal); the first one opens. Star one to fix it."
  - It is shown in the file bar's existing warnings area and in Settings.
  - Starring any file, or clearing, removes every other marker, which
    repairs it.
  - The repair is never automatic, because a GET must not write.

**D7. A deleted or renamed default needs no special handling.**
- The marker travels with the file, so a rename keeps the default.
- A deleted file takes its marker with it, and the tab falls back to D2's
  no-default path.
- This plan therefore needs no "missing default" state.

**D8. Save As never copies the marker.** A new file starts out non-default,
so Save As cannot create a second default. The server enforces this (D12).

**D9. A file that is the default and fails to open** (a parse error) shows the
engine's sentence as `fileError` does today. The tab then falls back to D2's
no-default path. The marker is left alone, and `lastProjectionId` is not
forgotten.

### Writing the marker

**D10. One route changes the default: `PUT /api/projections/default`,
`{id: string | null}`.**
- With `id`, it marks that file and unmarks every other marked file.
- With `null`, it unmarks every marked file.
- It runs under `import_writes`, after `validate_id`. It uses a **headers**
  discovery (`Discovery::in`), because it has to find every marked file. That
  costs the same 4 KiB per projection file the listing already reads.

**D11. Every edit is a surgical header edit, made by a new pure core function:
`set_default_marker(text, on: bool) -> Option<String>`.**
- It returns `None` when nothing needs to change, and then nothing is written.
- **Adding:** insert `; default-scenario: true` + the file's newline right after the
  last recognised line of `header_span`. If the file has no header (a
  hand-written projection file), insert it at offset 0.
- **Removing:** delete exactly the line or lines with key `default-scenario` inside
  `header_span`.
- Every other byte is unchanged, including line endings, `#` versus `;`, and
  the header's own order.
- `updated:` is **not** bumped. Starring a file is not an edit of the plan,
  and leaving `updated:` alone keeps the edit to exactly one line.
- `is_header_comment` and `parse_header` both learn the key `default-scenario`. Without
  that, a marker placed anywhere but last would cut the header span short
  (see "The scenario file header").

**D12. A normal save carries the marker from disk, not from the client.**
- `update_scenario` already parses the current on-disk text (`doc`). It sets
  the scenario's `is_default` from **that** text's header before rendering.
  `create_scenario` forces it to `false` (D8).
- So the marker is **server-owned header metadata**. No client can add or
  drop it through a save, and an older client that knows nothing about it
  preserves it.
- `is_default` is **not** added to `ScenarioIn`, which would break older
  clients through `deny_unknown_fields`. It is an output-only field on
  `WireScenarioFile`.

**D13. Write order: pre-flight everything, unmark the others, then mark the
target.** One file write is atomic (`Vault::replace`). Several are not, so the
order decides what a partial failure leaves behind.
1. **Pre-flight, with nothing written.** Resolve the target. Refuse with 400
   if it is not a projection file (D5), and with 400 decision 5's sentence if
   it is read-only. Read every affected file, compute its new text with
   `set_default_marker`, and hold its `Revision`. Any read or parse failure
   stops here and writes nothing.
2. **Unmark the other files**, in id order, each with `Vault::replace` pinned
   to its own pre-flight `Revision`.
3. **Mark the target** last, pinned the same way.

Why this order:
- A failure part-way leaves **too few** markers rather than too many. "No
  default" falls back to today's behaviour, which is safe and visible: the
  star is not filled and an error is shown.
- Marking first and then failing to unmark would leave two markers, and D6
  could then open the **wrong** file on the next launch with no error in
  sight.
- The pre-flight makes a failure at step 3 rare, because only I/O or a
  concurrent edit can still cause one.

**D14. Each file is checked for concurrent edits, but the client does not
have to hold a revision for it.**
- The server's own read → `Vault::replace(expected)` catches anything that
  changes a file between pre-flight and write (`VaultError::Changed` → 409).
- It never edits bytes it did not just read, so it cannot overwrite an
  external edit.
- The client's held revision of the open file is reconciled from the
  response (D16), not sent in the request. That is what lets Settings change
  the default while the Projections tab holds a dirty file.

**D15. Outcomes and errors**, pinned in `tests/error_surface.rs` where the SPA
can reach them:

| Case                                    | Status  | Body                                                                                                                 |
| --------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------- |
| malformed id                            | 400     | `validate_id`'s existing sentence                                                                                    |
| id not found / no journal open          | 404     | `unresolved(id)`'s existing sentence                                                                                 |
| not a `projection-*.journal`            | 400     | "Only a scenario file Ledgeline saved (projection-….journal) can be the default. Use Save as to make one."           |
| part of the main journal                | 400     | the decision-5 refusal sentence the save already uses                                                                |
| editing disabled                        | 501     | `editing_disabled()`'s existing sentence                                                                             |
| changed on disk before pre-flight ended | 409     | "{id} changed on disk while the default was being set. Nothing was written; try again."                              |
| failed after step 2 began               | 409/500 | "Removed the default mark from {ids}, but could not mark {id}: {reason}. No scenario is the default now; try again." |

Ids are relative, so naming them in a body is allowed. `{reason}` is
path-free (`path_free`).

**D16. A success answers with every file it changed.**

```json
{"changed": [{"id": "projection-a.journal", "before": "<rev>", "after": "<rev>", "isDefault": false},
             {"id": "projection-b.journal", "before": "<rev>", "after": "<rev>", "isDefault": true}]}
```

`before` and `after` are fingerprint tokens. `changed` is empty for a no-op.

### Unsaved edits (the star stays enabled when dirty)

**D17. Starring a dirty file writes only the marker. The client then moves
its held revision forward, so neither the star nor the pending edits clobber
the other.** Step by step:

1. The user has file B open with unsaved edits. The client holds revision
   `R1`, which matches the disk.
2. The user stars B. The server reads B (`R1`), inserts the one line and
   writes `R2`. It answers `changed: [{id: B, before: R1, after: R2,
   isDefault: true}, …]`.
3. `fileStore` sees `currentId === B` and `revision === before`, so it sets
   `revision = R2` and `isDefault = true`. The scenario, `dirty` and the edits
   are **untouched**. The scenario holds no marker (D12), so there is nothing
   to merge.
4. The next Save sends `R2`. The server checks that `R2` matches the disk,
   reads the marker from the disk (`true`), and renders the user's edits with
   `; default-scenario: true` in the header. Both survive.

Edge cases:
- **Same rule from Settings or for an un-star.** Any `changed` entry for the
  open file whose `before` equals the held revision is reconciled the same way,
  whichever screen sent the `PUT`.
- **`before` does not match the held revision.** Something else changed the
  file first. The client keeps its stale revision on purpose, so the disk poll
  reports "changed on disk" and a Save gets 409, exactly as for any external
  edit today.
- **The disk poll during the `PUT`.** `fileStore.settingDefault` is a flag that
  `checkDisk` treats like `saving`: the poll is skipped, and `superseded()` also
  treats it as a reason to discard a result.

**D18. A seeded scenario that was never saved cannot be starred,** because
there is no file to mark. The tooltip says "Save this scenario to a file
first."

### Wire

**D19.**

| Route / type                       | Change                                                                                         |
| ---------------------------------- | ---------------------------------------------------------------------------------------------- |
| `GET /api/projections` file        | adds `isDefault: boolean` (the raw marker; always `false` for a file whose header is not read) |
| `GET /api/projections` index       | a conflict (D6) adds a sentence to the existing `warnings`                                     |
| `GET`/`PUT /api/projections/{*id}` | `WireScenarioFile` adds `isDefault: boolean` (output only)                                     |
| `PUT /api/projections/default`     | new. Body `{id: string \| null}` (`deny_unknown_fields`). Answers D16                          |

- **Feature detection.** If `isDefault` is absent from the listing's files,
  the engine is older: the decoder sets `defaultSupported: false` and the star
  and the Settings control are hidden. An empty listing has nothing to star,
  so the case where detection is impossible does not matter. A `PUT` is never
  sent to an older engine, where it would land on `{*id}` and get a confusing
  400.
- **The effective default** is computed in TypeScript from the listing: the
  first file (in listing order) with `isDefault`. That matches D6, and the
  rule is pinned on both sides by tests.

### UI

**D20. The star sits right after the scenario select, before Save.**
- It is `btn btn-ghost btn-sm btn-square` with an inline SVG star, and
  `data-testid="projection-default-toggle"`.
- **Outline star:** `aria-pressed="false"`, label "Open this scenario by
  default".
- **Filled star** (in `text-warning`) when the open file is the effective
  default: `aria-pressed="true"`, label "Default scenario — click to stop
  opening it by default". Clicking it clears the default.
- **Enabled when dirty** (D17).
- **Disabled, with the reason as `title`:**
  - seeded and never saved (D18);
  - not a projection file, or read-only (D5);
  - `editable === false`: "Ledgeline can't write to this journal's folder";
  - `busy`.
- **Hidden** when `!defaultSupported` or `!fileStore.available`.

**D21. The picker marks the default** by appending " (default)" to its
`<option>` text.

**D22. Settings gets a fifth tab, "Projections"** (`?tab=projections`, key
<kbd>5</kbd>). It holds one control, "Default scenario":
- **A select** whose first option is "None — reopen the last scenario, or
  start from your journal", followed by the eligible files (D5) only.
- **A change applies immediately** (`PUT`). The select is disabled while the
  request is in flight, and an error appears inline.
- **The D6 conflict warning**, if any.
- **Read-only** (`editable === false`): the select is disabled, with the
  sentence `docs/settings/index.md` already uses for read-only tabs.
- The option rendering is shared with the file bar through
  `ScenarioOptions.svelte`.

**D23. The default's state lives in `fileStore`**, which already owns the
index.
- After a `PUT`, `fileStore` updates its copy of the listing's `isDefault`
  flags from `changed`, reconciles the open file (D17), and reloads the index
  in the background, as a save already does.
- **After a failure**, it reloads the index, because a partial failure (D13)
  changed some files.

### Not in this plan

- The desktop localStorage reset is tracked in TODO.md under "Known issues".
- A default for a file that is not a projection file (D5).
- A default per window, depth or reading. The URL carries those.

## Phase 1 — The marker and the wire (PR 1)

No visible change. The SPA gets the decoders and the client method, so the
live integration suite covers the new shape.

**Code: `crates/ledgeline-core`**
- `projections.rs`: `Scenario.is_default: bool` (default `false`).
- `projections/discovery.rs`:
  - `parse_header` reads `default-scenario` (`true`, case-insensitive) into
    `Header.default: bool`, first wins;
  - `DiscoveredProjection.is_default: bool`, which is always `false` for a
    file whose header is not read;
  - add `default_conflict(&Discovery) -> Option<String>` for the D6 warning.
- `projections/serialize.rs`:
  - `is_header_comment` accepts `default-scenario`;
  - `render_header` writes `; default-scenario: true` **last**, when `is_default`;
  - the read (:476-484) sets `is_default` from the header;
  - new `pub fn set_default_marker(text: &str, on: bool) -> Option<String>`
    (D11).

**Code: `crates/ledgeline-server` (`projections_api.rs`, `lib.rs`)**
- `WireProjectionFile.is_default`.
- `WireScenarioFile.is_default`, filled by `open_file` and by both save
  returns.
- `index` pushes the D6 warning.
- `update_scenario` sets `scenario.is_default` from `doc`'s header (D12), and
  `create_scenario` forces `false`.
- New `set_default` handler, in D13's three steps:
  - `SetDefaultRequest { id: Option<String> }` uses `deny_unknown_fields`;
  - the answer is `WireDefaultChange { changed: Vec<WireChangedFile> }`.
- `lib.rs`: `.route("/api/projections/default", put(projections_api::set_default))`,
  placed with `run`/`render`/`seed` above `{*id}` and under the token guard.
  Add one line to the placement comment.

**Code: `web/`**
- `projections/types.ts`:
  - `ProjectionFile.isDefault`;
  - `ProjectionIndex.defaultSupported`;
  - `ScenarioFile.isDefault`;
  - `DefaultChange`.
- `api/nativeDecode.ts`:
  - files decode `isDefault`, and its absence means `defaultSupported: false`;
  - `decodeScenarioFile`: an absent `isDefault` is `false`;
  - `decodeDefaultChange`.
- `api/native.ts`: `setProjectionDefault(id | null)` via `mutate`.

**Tests (Rust, core)**
- `set_default_marker`, table-driven:
  - adding to a full header, with LF and with CRLF;
  - adding to a `#`-comment header;
  - adding to a file with no header;
  - adding when already on, or removing when absent, gives `None`;
  - removing deletes the line and nothing else, checked byte-for-byte against
    the input with the line deleted;
  - removing two duplicate lines;
  - after an add, `header_span` covers the marker and `parse_header` reads it.
- `parse_header`:
  - `TRUE` and `True` count; `false`, `yes` and an empty value do not;
  - a marker below the leading block is ignored, as `projection:` already is.
- Round trip:
  - a file with the marker, saved with `is_default: true`, is byte-identical,
    so a no-op stays a no-op;
  - rendering with `is_default: false` drops the line.
- **Older-build fixture:** a header whose `default-scenario:` line is last. With
  `is_header_comment` as it is on `main` (simulated by a test-only predicate,
  or by a fixture written by the old writer), the line survives as a comment
  piece.
- **hledger ignores it:** in the parity suite that shells out to hledger, a
  scenario with and without `; default-scenario: true` gives identical
  `hledger print` output. If hledger is unavailable the test skips, as the
  others there do.

**Tests (Rust, `projection_endpoints.rs`, tempdir journals)**
- **Marking and unmarking:**
  - the listing's `isDefault` follows the marker;
  - PUT `{id: B}` with A marked: A loses the line, B gains it, and every other
    byte of both files is unchanged;
  - `changed` lists both, with revisions that match the new bytes;
  - PUT `{id: null}` unmarks everything;
  - PUT on the file that is already the only default gives `changed: []` and
    leaves the mtime unchanged.
- **Conflicts:** two marked files produce the warning, and the listing marks
  both.
- **The save path (D12):**
  - with a marked file, a save without the marker in the request keeps it;
  - Save As (create) never writes it, even when the source was marked.
- **Refusals:**
  - other-journal → 400 and read-only → 400, with nothing written;
  - an unresolved id → 404;
  - editing disabled → 501, while GET still reports `isDefault`.
- **Partial failure:**
  - change B on disk between pre-flight and step 3 (a test hook or an
    injected vault), and the answer is the D15 sentence, with A unmarked and B
    untouched;
  - a changed file during pre-flight writes nothing at all.
- **Plumbing:**
  - the 401 sweep (:1537 and :2507 lists) gains the new route;
  - `{id, extra}` → 400.
- `error_surface.rs`: one case per D15 row that the SPA can reach.
- `native_wire_golden.rs`: regenerate `projections-asset` (it gains
  `isDefault: false`).

**Tests (vitest)**
- `nativeDecode.test.ts`:
  - `PROJECTION_INDEX` files gain `isDefault`, and its absence gives
    `defaultSupported: false`;
  - `isDefault: "yes"` throws;
  - `decodeScenarioFile` defaults an absent key to `false`;
  - a new `DECODERS` triple for `decodeDefaultChange`.
- `nativeLive.integration.test.ts`: the live engine serves committed fixtures,
  so **this test must not write.**
  - The index decodes with `defaultSupported: true`.
  - `PUT {id: "../x.journal"}` → `ValidationError`.
  - `PUT {id: "sample.journal"}` → `ValidationError`. It is not a projection
    file, so it is refused in pre-flight, before anything is written.

**Done when:** `cargo test --workspace`, `just test`, **`just
test-integration`** (the `/api` shape changed) and `just lint` all pass.

## Phase 2 — The Projections tab (PR 2)

**First:** write the failing tab-revisit test (below) and confirm it fails on
`main`.

**Code**
- `projections/opening.ts` (new, pure):

  ```ts
  export function effectiveDefault(files: readonly ProjectionFile[]): ProjectionFile | null;
  export function openingSteps(defaultId: string | null, lastId: string | null, available: boolean): {id: string; source: "default" | "last"}[];
  ```

- `projections/fileStore.svelte.ts`:
  - `currentIsDefault`;
  - `settingDefault` (D17), with `checkDisk` and `superseded()` honouring it;
  - `setDefault(url, id | null)` applies D23 and D17's reconciliation;
  - `adopted(file)` records `file.isDefault`.
- `routes/projections/+page.svelte`:
  - `start()`:
    1. `await ensureIndex`;
    2. **`if (scenarioStore.adopted) return;`** (D3);
    3. try each opening step;
    4. otherwise `ensureSeeded`.
  - `openFile(url, id, source = "last")` forgets only when
    `source === "last"`. On a default failure, `fileError` is "Your default
    scenario couldn't be opened: {sentence}" (D9).
  - Update the header comment (:30-34).
  - `toggleDefault()` routes failures to `fileError`.
- `projections/ui/ScenarioOptions.svelte` (new): the `<optgroup>`s plus the
  " (default)" suffix. `ScenarioFileBar` uses it in place of :88-101.
- `projections/ui/ScenarioFileBar.svelte`: the star (D20), and the props
  `isDefault`, `canBeDefault` + `defaultBlocker`, `defaultSupported` and
  `onToggleDefault`.

**Tests (vitest)**
- `opening.test.ts`:
  - `effectiveDefault` picks the first marked file in listing order, with zero,
    one or two marked;
  - `openingSteps` is checked over (default none / set) × (last none / same /
    different) × `available`.
- `fileStore.svelte.test.ts`:
  - **D17 reconciliation:** dirty with held `R1`, a `PUT` answering
    `before: R1, after: R2` gives held `R2`, `dirty` still true and the
    scenario unchanged;
  - with `before ≠ held`, the revision is not moved;
  - `checkDisk` while `settingDefault` returns `skipped`;
  - after a failure, the index is reloaded.
- `ScenarioFileBar.svelte.test.ts` (new):
  - the star is hidden when `!defaultSupported`;
  - it is **enabled when dirty**;
  - it is disabled for seeded, other-journal, read-only and non-editable,
    each with its own tooltip;
  - `aria-pressed` follows the default;
  - the " (default)" suffix is shown.
- A page-level component test with an injected fetch:
  - **Revisit keeps unsaved edits:** open A, edit, unmount, remount. The
    edited scenario is still there, `dirty` is true and A is not fetched
    again. This test fails on `main`.
  - **The default wins on the first mount:** with B marked and
    `lastProjectionId` A, B is opened and A is never requested.
  - **A default that fails to parse:** a 400 on B, then A opens, `fileError`
    names B and `lastProjectionId` is still A.
  - **No default:** the same requests as today in all three of today's cases.
  - **Star while dirty, then save:** the `PUT /api/projections/{B}` body
    carries `R2`, and the response is adopted or marked saved as today.

**e2e** (`web/e2e/projectionDefault.e2e.ts`, new; specified here, run by
Patrick)
- **A new scratch engine on :5095:**
  - `rm -rf ../target/e2e-projection-default && cp -R ../fixtures/projection-default ../target/e2e-projection-default && exec … --server ../target/e2e-projection-default/main.journal --port 5095 …`;
  - the env is the same as the other engines.
- **The fixture** is `fixtures/projection-default/`:
  - `main.journal`;
  - `budget.journal`, included by main, so it is read-only;
  - `projection-alpha.journal` and `projection-beta.journal`, each with a name
    and one distinct `~` rule.
- **The spec:**
  1. On `/projections`, the seeded scenario's star is disabled with the
     "Save … first" title.
  2. Pick budget: the star is disabled (read-only). Pick Alpha and star it:
     it is pressed and the option reads "Alpha (default)". The file on disk
     contains `; default-scenario: true`, read with `fs` from the spec.
  3. Pick Beta, go to `/reports` and come back: Beta is still shown (D2).
  4. Edit Beta's rule, then star Beta **while it is dirty**. The "edited"
     badge stays, Alpha's file has lost its marker and Beta's has gained it.
     Save: the save succeeds with no 409, and Beta's file holds both the edit
     and the marker (D17).
  5. `page.reload()`: Beta is selected (fresh mount).
  6. A new `page` with fresh storage, standing in for a relaunch: Beta is
     selected.
  7. Use `fs` to add `; default-scenario: true` to Alpha's header by hand, then
     reload. The conflict warning shows, and Alpha, which is first by id,
     opens. Star Beta and the warning is gone.
  8. Un-star. Reload, and the behaviour is today's (the last file opened).

## Phase 3 — Settings tab and docs (PR 3; may fold into PR 2)

**Code**
- `settings/params.ts`: add `"projections"` to `SettingsTab`, to `TAB_ORDER`
  (last, so keys 1–4 do not move) and to `TAB_LABELS`.
- `projections/ui/ProjectionSettingsPanel.svelte` (new), per D22.
- `routes/settings/+page.svelte`: add the branch and update the header
  comment.
- `TODO.md`: remove the "Default projection" item.

**Tests (vitest)**
- `settings/params.test.ts`: `projections` round-trips.
- `ProjectionSettingsPanel.svelte.test.ts`:
  - "None" is selected when there is no default;
  - the list offers only eligible files;
  - a choice sends one `PUT`;
  - "None" sends `{id: null}`;
  - the conflict warning is shown;
  - read-only disables the select;
  - a failed `PUT` shows the sentence and keeps the previous selection.
- `SettingsTabs`: <kbd>5</kbd> selects Projections.

**e2e:** in `settings.e2e.ts` (:5099, read-only), `?tab=projections` renders
the panel. In `projectionDefault.e2e.ts`, after step 2, Settings →
Projections shows Alpha selected. Choosing Beta there, then going back to
Projections, shows the star filled while Beta is open.

**Docs**
- `docs/projections/file-format.md`: the header table gains `default-scenario:`. Say
  that it is ignored by hledger, written and removed by the star, and that at
  most one file should carry it, with the first by name winning.
- `docs/projections/index.md`: a new subsection, "Your default scenario". It
  covers:
  - the star, and that it works while you have unsaved edits;
  - what opens on launch versus on returning to the tab;
  - only projection files you can save can be the default;
  - Save As does not copy it;
  - deleting or renaming the file;
  - the conflict warning.
- `docs/settings/index.md`: "five tabs", a new row and `?tab=projections`.
- A screenshot: `docs/projections/images/default.png`. Use mbr-compatible
  callouts, with no `[!INFO]`.

## Sequencing

1. **PR 1** (Phase 1). Run `just test-integration` before pushing.
2. **PR 2** (Phase 2), then **PR 3** (Phase 3), which depends on PR 2's
   `fileStore.setDefault` and `ScenarioOptions`. Fold PR 3 into PR 2 if PR 2
   lands quickly.

Worktree agents branch from `main`. Create worktrees from HEAD and warm
`target/` with `/bin/cp -Rc`. Agents do not run Playwright. If other worktrees
hold :5099/:4173, wait for them; never kill them.

## Risks

- **Older Ledgeline builds.** An older build reads `; default-scenario: true` as an
  ordinary comment after the header, because the line is written last and
  older builds don't recognise the key. A save then keeps it as a comment
  piece (plan 31) rather than dropping it. When the new build reads it again,
  `header_span` takes it back into the header, because a blank line is
  absorbed when a recognised line follows. Pinned by the older-build fixture
  test.
- **Several files are written for one click.** A partial failure leaves "no
  default" (D13). That is visible, safe and fixed by clicking again. Two
  markers can only come from hand edits or sync merges, and D6 makes the
  outcome deterministic and visible.
- **The client misses the `PUT` response** (a dropped connection) while a file
  is dirty. The held revision is then stale, the disk poll shows "changed on
  disk", and Save gets 409. This is the same recovery as for any external
  edit, and the unsaved edits are at risk only if the user chooses to reload.
  It is rare on localhost.
- **Sync conflicts.** iCloud or git may merge two machines' stars into two
  markers. D6 handles it, and the warning tells the user.
- **`updated:` is not bumped by a star.** A user who sorts by `updated:`
  won't see a star as an edit. This is intended (D11).
- **The disk poll on other open tabs.** If a second window has the
  un-starred file open, its poll sees a new revision. It adopts the file when
  clean, or shows "changed on disk" when dirty. This is correct, if noisy.
  There is one window today.

## Resolved with Patrick on review (2026-11-09)

1. **Storage.** The default is a header line in the scenario file, not a
   Ledgeline-only file in the journal folder, so everything there stays plain
   hledger (D4).
2. **The star stays enabled with unsaved edits.** Only the marker is written,
   and the held revision is reconciled (D17).
3. **The localStorage reset** is tracked in TODO.md under "Known issues" and
   is not part of this plan.
4. **Spelling:** `; default-scenario: true`, not `; default: true`, so it is
   not confused with the "Default cash account" (`; cash:`) (D4).
5. **Eligibility:** only `projection-*.journal` files that the tab can write
   can be the default. A budget file the main journal includes cannot be, and
   Save as is the workaround (D5).
6. **Conflict warning:** when several files carry the marker, the warning is
   shown on the Projections tab and in Settings (D6, D22).

## Open questions for Patrick

None.

## Status

- Not started.

## Contract amendments made during implementation

(none yet)
