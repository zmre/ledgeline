# ledgeline-ffi

The UniFFI library an iOS app links to run Ledgeline. It runs the desktop
server's router in-process behind a custom URL scheme, over a folder (or one file)
the user granted from Files or iCloud Drive. Not published; internal to this
workspace. Plan: `plans/26-platform-readiness.md` (A3).

The template for the Swift side is mbr's iOS shell
(`mbr-markdown-browser/apple/ios/`, with its `build.sh`, and
`design/ios-architecture-research.md` for the reasoning). This crate is
modelled closely on mbr's `crates/mbr-ffi` (`src/mobile/`). The Swift shell
itself is out of scope here.

## Building

- Host tests: `cargo test -p ledgeline-ffi --features mobile` (part of
  `just engine-test` and the flake's `tests` check).
- iOS type-check: `just ios-check` (`scripts/ios-check.sh`). It needs macOS,
  Xcode, and the dev shell's iOS std targets (`rustToolchainDev` in
  `flake.nix`). `scripts/ios-check.sh build` builds the release staticlib,
  `target/aarch64-apple-ios[-sim]/release/libledgeline_ffi.a`. The script
  sends only the target-side C compiles (rustls' `aws-lc-sys`) to Xcode's
  SDK. Everything else keeps the nix toolchain.
- Bindings: proc-macros only, so there's no UDL and no build script. Generate
  the Swift with `uniffi-bindgen` 0.31 from the built library (`uniffi-bindgen
  generate --library libledgeline_ffi.a --language swift`), as mbr's
  `build.sh` does.

The UniFFI exports compile on Apple targets only (`target_vendor = "apple"`,
which includes macOS, so the host tests exercise them there). Elsewhere the
crate is plain Rust.

## Exported API

```text
LedgelineApp.open(rootPath, mainFile, cacheDir?, coordinator?) throws -> LedgelineApp
LedgelineApp.openFile(path, coordinator?) throws -> LedgelineApp
app.handle(method, url, headers: [Header], body: Data) async throws -> LedgelineResponse
app.refresh() async throws -> RefreshSummary
app.accessToken() -> String
app.tokenInitScript(origin?) -> String
app.rootPath() / mainFile() / isSingleFile() / isEditable()
response.status() / headers() / nextChunk() async / readAll() async
setMaterializeDatalessFiles(Bool) / materializesDatalessFiles()   // debug toggle
```

Errors are `MobileError`: `Open`, `NotDownloaded { paths }`, `NeedsFolder`,
`InvalidRequest` and `Internal`. HTTP failures (401, 409, 503 and so on) come
back as responses, not errors.

## The Swift-side contract

### Scheme handler → `handle()`

- Register a `WKURLSchemeHandler` for `ledgeline` and load the SPA from
  `ledgeline://localhost/` (the `MOBILE_ORIGIN`).
- For each `WKURLSchemeTask`, pass the method, full URL, headers and body to
  `handle()`. Send `status` and `headers` back as an `HTTPURLResponse`, then
  stream `nextChunk()` until it returns `nil`. If a chunk errors, fail the
  task.
- `Accept-Encoding` is stripped, so bodies are never compressed. WebKit doesn't
  decode a scheme handler's bodies.

### The token

The bearer token is still required in-process (`Security::in_process`). A
scheme handler receives every request its web view makes, including one from
a foreign page that ends up loaded in it. Install `tokenInitScript()` as a
`WKUserScript` (`.atDocumentStart`, main frame only). It sets
`window.__LEDGELINE_TOKEN__` only on the Ledgeline origin, exactly as the
desktop WebView gets it. Also keep the web view on `ledgeline://localhost` in
`decidePolicyFor` and open other links externally.

### `FileCoordinator`: writes, conditionally

Implement it with `NSFileCoordinator`. Reads never cross the boundary, because
Rust reads with `std::fs`. Every call comes from a Ledgeline background thread
and may block. Paths are absolute.

- **`coordinatedReplace(path, contents, expectedLen, expectedHash)` is
  conditional.** Inside the coordinated `.forReplacing` write block:
  1. Refuse with `CoordinatorError.Changed` if `path` is missing, is a
     symlink, or isn't a regular file.
  2. Read the file, then compare its byte length to `expectedLen` and its hash
     to `expectedHash`. On any mismatch, throw `Changed` and write nothing.
  3. Otherwise, replace the file atomically (item-replacement directory, then
     `replaceItemAt`).

  The hash is **FNV-1a 64-bit** as **16 lowercase hex digits**, zero-padded.
  Start with `h = 0xcbf29ce484222325`. For each byte: `h ^= UInt64(b);
  h = h &* 0x100000001b3`. Test vectors:

  | Input      | Hash               |
  |------------|--------------------|
  | `""`       | `cbf29ce484222325` |
  | `"a"`      | `af63dc4c8601ec8c` |
  | `"foobar"` | `85944171f73967e8` |

  This is how Ledgeline's external-change guarantee crosses the seam: an edit
  synced in from another device between the app's read and its write gets a
  409, never a silent overwrite. Rust runs the same check first, so a stale
  edit is usually refused before Swift is called. Only the coordinated block
  makes the check atomic with the write. Identity (inode) is not compared on
  this path, because providers replace inodes when they sync.
- **`coordinatedCreate(path, contents)`** is exclusive. If anything already
  exists at `path`, throw `AlreadyExists`.
- **`coordinatedCreateDirectory`** and **`coordinatedRemove`** do what their
  names say.
- **`requestDownload(paths)`** starts the downloads and returns at once. For
  iCloud use `startDownloadingUbiquitousItem(at:)`; for other providers, a
  coordinated read on a background queue.

`coordinator: nil` writes with plain `std::fs`. That's only right for a folder
inside the app's own container.

### Not-downloaded files

Dataless files (`SF_DATALESS`) and legacy `.Name.icloud` stubs are reported as
not downloaded. Ledgeline's runtime threads also set `setiopolicy_np(…
MATERIALIZE_DATALESS_FILES_OFF)`, so a read never blocks on the network. How
it shows up:

- **At open:** `MobileError.NotDownloaded(paths)`. The downloads have been
  requested, so open again when they land.
- **During a request:** a `503` with `Retry-After: 2`.
- **On refresh:** `RefreshOutcome.Pending` with `notDownloaded`. The open
  editor isn't disturbed.

### Bookmarks and opening

- Keep a security-scoped bookmark for the granted folder (or file). Resolve
  it, call `startAccessingSecurityScopedResource()`, then `open()` or
  `openFile()`. Stop accessing only after the `LedgelineApp` is dropped.
- `openFile()` sees only that file. A journal with `include`s fails with
  `NeedsFolder`: ask for the folder and `open()` it instead.
- The Rust recents store (`ledgeline::recents`) holds opaque tokens. To have
  Ledgeline remember journals, store the bookmark *id* there with
  `RecentLocation::new(id)`. The bookmark data itself stays with the app.

### Refresh on foreground

There's no file watcher. Call `refresh()` on `scenePhase == .active`, on
pull-to-refresh, and a few seconds after a `Pending` outcome:

- `Unchanged` is cheap: each file is read and compared, and nothing is
  reparsed.
- `Reloaded` means a new snapshot is live. The SPA's own 30 s poll picks it
  up, or reload the page (`evaluateJavaScript("location.reload()")`).
- `Failed` means the journal no longer parses (a bad edit synced in). The last
  good snapshot is still served. A later clean refresh rebinds the editor;
  `editable` says whether writes are live.

### App directories

`cacheDir` (the app's `Caches/`) holds the staging area for dropped
statements. There's no config directory: nothing in the preferences store
applies to a build with no external tools.
