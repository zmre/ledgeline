# ![](web/static/ledgeline-icon.png) Ledgeline (hledger GUI)

A fast, local, privacy-centric desktop app for [hledger](https://hledger.org) plain-text accounting. Ledgeline is a **single binary** that opens a native window showing a modern, fast UI. It parses your journal file directly and reproduces hledger's numbers exactly.

I built this because I was dissatisfied with existing GUIs. They often hard code expectations for where files are and how they link. Or they're old and ugly. They rarely handle stocks well. If they even allow editing, it's problematic and buggy. I love the command line and editing in the terminal, but sometimes I want graphs and something pretty and ledgeline scratches that itch.

> [!WARNING]
> Disclaimer: I built this for myself and based it on patterns I've built by hand in the past (see [mbr](https://github.com/zmre/mbr-markdown-browser/)), but this project heavily leveraged AI for development.

## What it does

- **Journal view** with live filtering and an insights panel (pie / line charts, account-depth control).
- **Reports** — balance sheet, income statement, cash flow and net worth. Computed in Rust with exact
  decimal math and hledger parity. XLSX exports.
- **Budget** — your `~` periodic goals against what actually happened, as a period-summary envelope bar
  per category (year-to-date by default) — *and* an editor for the goals themselves. Add or change a
  weekly / monthly / quarterly / annual goal and Ledgeline writes it back into your journal as an
  ordinary hledger `~` rule, in the file your goals already live in. Setting one shows the last three
  periods of that account's actual activity beside the amount box, so a number is set against history
  rather than from memory:

  ```journal
  ~ monthly  household budget
      (expenses:food)      $400
      (expenses:bus)        $20
  ```

  Income is typed as a magnitude and written the way hledger wants it (`(income:interest)  $-1200`).
  An edit rewrites only the amount it names — alignment, comments and everything else come out of the
  file exactly as they went in — and a rule Ledgeline cannot rewrite safely is shown read-only with the
  reason rather than guessed at. No budget rules yet? One button writes a `budget.journal` beside your
  journal and includes it. See **[docs/budget.md](docs/budget.md)**.
- **Money-flow diagrams** on the income statement: one Sankey above Revenue showing where the period's income came from and which accounts it landed in, one above the expenses showing which accounts paid and what each category was spent on, so card spending shows the card. Each is a decomposition of the box below it, not a second calculation: the ribbon widths add up to the figures already printed there, and where they cannot, the panel says `Showing $X of $Y` rather than implying they do. Withheld tax is attributed to gross pay and not to the bank account, which is the case a naive debit-against-credit pairing gets wrong. Both panels are collapsible, follow the report's date range, and are absent from the XLSX export. See **[docs/income-statement.md](docs/income-statement.md)** for the attribution rule, the links that are not drawn, and why colour tracks the account.
- **Balance sheet** — three boxes (assets / liabilities / equity), every line valued to one number in your
  base currency so a portfolio reads as money rather than as a column of share counts. Lines are *groups*
  ("Cash and cash equivalents", "Investments"), collapsed by default and expandable to the accounts behind
  them. Tag any account with `bsgroup:` to put it on a line of your choosing — the tag inherits to
  sub-accounts exactly like `type:` does:

  ```journal
  account assets:property:house    ; type: A, bsgroup: Property
  account liabilities:mortgage     ; type: L, bsgroup: Long-term debt
  ```

  Equity carries a computed **Retained earnings** line, so `assets = liabilities + equity` ties out and the
  balance check is a real journal-integrity signal rather than decoration. See
  **[docs/balance-sheet.md](docs/balance-sheet.md)** for the grouping rules, valuation, and why the check
  has a tolerance.
- **Holdings** — two sub-tabs. **Stocks**: average-cost basis, unrealized gain (all-time /
  year-to-date / trailing-12-months), value-over-time, per-symbol names from `commodity` directives,
  partial portfolio totals; XLSX export. **Other**: the assets that are neither securities nor cash —
  a house, a car, a partnership interest — one row per account with its value, cost and change over
  the same window. Which tab an account lands on is mechanical (does it hold a non-currency
  commodity?) and overridable per account:

  ```journal
  account assets:property:home  ; type: A, holdings: other
  account assets:receivable     ; type: A, holdings: none
  ```

  A holding may span several accounts. The common cost/market split is rolled into one row, and
  `valuation:` says which side each account is on — so Cost stays what you actually paid and the
  difference is the unrealized gain:

  ```journal
  account assets:home:cost        ; type: A
  account assets:home:unrealized  ; type: A, valuation: unrealized
  ```

  See **[docs/holdings.md](docs/holdings.md)** for both tags, how several accounts become one
  holding, the ways a non-stock asset changes value, and why the three reports read prices from
  different places.
- **Imports** — finds the CSV import rules files (`*.rules`) beside your journal and edits them in a
  friendly form instead of a text box: date format, the CSV column → field mapping (labelled with your
  data file's real headers and sample values), the default accounts, and a reorderable list of `if`
  rules. Anything fancier than a plain OR rule — `if` tables, `&&`/`!` matchers, match groups — is shown
  read-only rather than rewritten, and saving preserves the rest of the file byte for byte.
- **Which books am I in?** — the app bar names the ledger on screen rather than showing the engine's
  URL, so someone who keeps several sets of books (a household, an LLC, a trust) can tell at a glance
  which one they are looking at. The name comes from the journal itself: if the main journal file's
  first non-empty line is a *short* comment — one to five words, and something more than a row of
  `=====` — that is the title; otherwise it is the name of the folder the main journal lives in.
  Nothing to configure, and a journal that says nothing still gets a sensible name.

  ```journal
  ; Acme Holdings LLC
  include accounts.journal
  ```

  Hover the name for the journal's file name and the engine's address — the same connection detail
  that used to occupy the corner, one hover away.
- **In-process, same-origin API** exposing both the hledger-web-compatible wire endpoints
  (`/version`, `/transactions`, `/prices`, …) and native report / holdings / budget JSON (`/api/*`) and
  edit endpoints.
- **No preconceived notions** on how accounts are setup or where things live or how they're organized.

## Install / Use

For now, requires [Nix](https://nixos.org) with flakes and works on Linux and Mac. In theory it should run on Windows, too, but I haven't tested that.  

On Mac, there's a native application bundle.  If there's demand (submit an issue), I'll build releases and maybe even publish them places.

**To run it directly in Nix**:

```sh
nix run --accept-flake-config github:zmre/ledgeline -- ~/finance/2026.journal   # opens the desktop window on the specified journal (or don't specify and you can open from inside the app)
```

`--accept-flake-config` opts you into our [binary cache](#binary-cache-skip-the-build) so this
downloads rather than compiles. Drop it if you'd rather build everything yourself.

**Install the binary + app** into your Nix profile:

```sh
nix profile install github:zmre/ledgeline
# macOS → installs bin/ledgeline (on PATH) AND Applications/Ledgeline.app
# Linux → installs bin/ledgeline plus a desktop entry and icons
# then:  ledgeline ~/finance/2026.journal        # or launch Ledgeline.app
```

On Linux this is what registers Ledgeline with your application launcher and
associates `.journal` / `.hledger` / `.ledger` files with it — a bare `nix build`
leaves the desktop entry in `./result`, where no launcher looks.

### Linux notes

The window opens with **no title bar and no menu bar**: the app draws its own
header, and an in-window GTK menu bar is out of place under a tiling Wayland
compositor. Press <kbd>F10</kbd> to bring both back (and again to dismiss them) —
that's where File → Open journal…, Open Recent and Quit live.

Shortcuts work in either state: <kbd>Ctrl</kbd>+<kbd>O</kbd> to open a journal,
<kbd>Ctrl</kbd>+<kbd>R</kbd> to reload, <kbd>Alt</kbd>+<kbd>←</kbd> /
<kbd>Alt</kbd>+<kbd>→</kbd> to go back and forward, and <kbd>Ctrl</kbd>+<kbd>Q</kbd>
to quit.

The Linux package wraps the binary so that nixpkgs' Mesa is available as a
**last-resort** EGL driver, appended to the search path and never substituted for
it, so your host driver still wins wherever there is one. Without it WebKitGTK
aborts its web process on any non-NixOS host and you get a blank window. This is
also why `.#ledgeline` (the bare, unwrapped binary that CI builds) is not the
thing to install.

**Build the macOS app bundle** to open or drag into `/Applications`:

```sh
nix build github:zmre/ledgeline        # or, in a local checkout: nix build
open result/Applications/Ledgeline.app # macOS — real UI embedded

just package-mac                       # macOS: a writable dist/Ledgeline.app to open / drag to /Applications
```

### Binary cache (skip the build)

Every push to `main` uploads its build products to
[`zmre.cachix.org`](https://app.cachix.org/cache/zmre), so the commands above can **download**
the Rust engine and the wry/tao GUI stack instead of compiling them. `flake.nix` already
declares the cache and its public key, but Nix ignores substituters coming from a flake it
doesn't trust — hence `--accept-flake-config` (it will also prompt you to accept
interactively).

To trust the cache permanently instead, put this in `~/.config/nix/nix.conf`:

```
extra-substituters = https://zmre.cachix.org
extra-trusted-public-keys = zmre.cachix.org-1:WIE1U2a16UyaUVr+Wind0JM6pEXBe43PQezdPKoDWLE=
```

or, on NixOS / nix-darwin, the equivalent `nix.settings.{extra-substituters,extra-trusted-public-keys}`.

> [!NOTE]
> `substituters` is a trusted setting: on a multi-user Nix install your user has to be in
> `trusted-users` for either method to take effect. Otherwise Nix prints
> `ignoring untrusted flake configuration setting 'extra-substituters'` and builds from source
> — correct, just slow.

CI pushes the Linux and macOS `ledgeline` binaries, the macOS distributable (`.#macDist` →
`Ledgeline.app`) and the crane dependency layer. Coverage is best-effort — Cachix
garbage-collects, so an older revision may well have been evicted. None of this is required:
a cache miss just means you build locally, same as if you'd never configured it.

## Development (or if you don't have nix)

```sh
direnv allow          # or: nix develop path:.
just --list           # available tasks
just engine-test      # cargo test over the workspace
just check            # SPA type-check + unit tests
just pre-push         # everything CI gates on, under 2 min warm; run before you push
cd web && bun run build && cd .. && cargo build --release && ./target/release/ledgeline ~/.../Ledger/main.journal
```

See **[docs/development.md](docs/development.md)** for the Nix + Crane build cache, the
`nix build .#{ledgeline,clippy,tests,fmt,macApp}` outputs, CI, and how the SPA is built and embedded.
See **[docs/imports.md](docs/imports.md)** for the CSV rules-file editor — the format-preserving
model, what it will and won't edit, and the guards on its write path.
See **[docs/balance-sheet.md](docs/balance-sheet.md)** for the balance sheet — the `bsgroup:` and
`bsterm:` tags, how untagged accounts are grouped, valuation, and the balance check's tolerance.
See **[docs/holdings.md](docs/holdings.md)** for the Holdings tabs — the `holdings:` and
`valuation:` tags, how several accounts become one holding, and what "change" measures against.

## Architecture

This spins up a local tokio axum API server and uses the native OS browser as a GUI window (via wry, part of the tauri project) hosting a svelte frontend app.  All assets are built into the single binary, which is pretty snappy.

