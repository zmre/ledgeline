# ![](web/static/ledgeline-icon.png) Ledgeline (hledger GUI)

A fast, local, privacy-centric desktop app for [hledger](https://hledger.org) plain-text accounting. Ledgeline is a **single binary** that opens a native window showing a modern, fast UI. It parses your journal file directly and reproduces hledger's numbers exactly.

I built this because I was dissatisfied with existing GUIs. They often hard code expectations for where files are and how they link. Or they're old and ugly. They rarely handle stocks well. If they even allow editing, it's problematic and buggy. I love the command line and editing in the terminal, but sometimes I want graphs and something pretty and ledgeline scratches that itch.

> [!WARNING]
> Disclaimer: I built this for myself and based it on patterns I've built by hand in the past (see [mbr](https://github.com/zmre/mbr-markdown-browser/)), but this project heavily leveraged AI for development.

## What it does

- **Journal view** with live filtering and an insights panel (pie / line charts, account-depth control).
- **Reports** — balance sheet, income statement, cash flow, net worth, and budgets (`~` periodic goals vs.
  actuals) — the budget view shows each category as a period-summary envelope bar (year-to-date by default).
  Computed in Rust with exact decimal math and hledger parity. XLSX exports.
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

## TODO

- fix: display issue where pie chart is not round, but oval when the window narrows horizontally or vertically.  Update: seems to be specific to linux as I can't reproduce on mac.
- test: lets try to understand performance on large repos by making a fixture with 10k transactions per year, 15 years, and around 200 commodities and 75 accounts
- chore: route bad `issection:` / `holdings:` / `valuation:` / `bsterm:` / `type:` tag values into Problems
  - a mistyped `issection:` currently fails the whole P&L request with a 400 naming the account and the
    valid codes, and `holdings:` now does the same to the Holdings tab. Right that it isn't silently
    dropped, wrong that one typo takes the tab down. Problems
    entries anchor to a `txnIndex` and an `account` directive has none, so this needs a wire field that
    allows a directive-anchored diagnostic plus an allow-list entry in the SPA's `normalize.ts`.
- check security csrf to be sure other apps/browser pages can't fetch financial data
- **Imports**
  - feat: quickbooks import handling
    - transaction matching and skipping
    - account mapping (prompt for unmapped) (aliases?)
  - feat: import drag/drop
    - command line options
    - fix styling of numbers issues; infected the entire ui now
  - feat: create new import rules files
    - take a csv file and make intelligent guesses on setup. we want intelligent mapping of headings, ask what account it is and default categorizations, figure out ordering of rows. detect separator, skip rows number, and encoding automatically. figure out date-format automatically. 
* **Editors**
  - Account List Editor
    - Most financial apps allow editing of the chart of accounts. We should detect where they live and allow editing. If there aren't any, we should create an accounts.journal and include it from the main file.
    - For each account, we should provide an editor for comments/notes, type, tags in general, and our special tags used in various reports
    - Lets put this under a Settings top level tab or gear icon. And lets figure out what else might go in here, like the number format stuff -- basically whatever hledger provides that we might want to set or edit
      - commodity, decimal-mark, tag list, and we should probably move aliases to here under "settings" too
  - rules editor ui improvements
    - we need to figure out a new rules editor approach because the current one is ugly, hard to find what you're looking for, very long vertically and not scannable
    - also: we can't do more sophisticated rules (with conditional logic in them) so we need to add that and figure out ways to display and edit them
    - perhaps instead of one giant form, we have display separate from edit and can therefore make this nicer
  - feat: edit / create budget
    - figure out where budget rules already exist and that's where we'll store new lines and update existing ones
    - if they don't exist, make a budget.journal file and include it from the main file (with a button press by user first)
    - Move Budget from Reports to its own top-level tab
- chore: Add screenshots and better descriptions to the readme
- feat: private AI integration
  - Need to make use of a per-user preference specifying the url for the AI and any necessary api keys or whatever
  - Need to make a way for the user to edit this in the app
  - Only show an AI icon if we have a successful connection; or maybe there's an AI chat icon that has a red dot over it if the configured url doesn't work, a green dot if it is working, and the icon is hidden if this isn't setup (under settings tab)
  - clicking the chat slides over a drawer
  - we need to build out a system prompt with information about the files in the folder. i think we need to allow a tool call to fetch files in the folder, but nothing else. and a tool call that could write to files, but with strict user approval checks.
  - i have mostly used ai functionality on my hledger repos when i want to use an external file -- often a pdf -- and produce some specific journal entries from it such as balance checks or setting up a partnership or doing fair value / nav adjustments
  - ultimately, the AI should interact with the user through the chat drawer. the user should allow it to see specific files or all files as they desire. user should be able to clear history.  ai should be able to request reading of files and propose changes to files.
  - i don't want to reinvent too much here.  and i'd like to use our API endpoints rather than any direct writing. maybe that's what we offer as tools is our api endpoints, but with per-session user approval?  i mean, if the AI is private, it's probably fine to offer up the read-only endpoints. the main thing is to gate writes, not reads.
- feat: stock price updates
  - basically my script, maybe ported into rust, for querying yahoo and updating a prices file. should try to figure out where prices already live and if it can't find anything, prompt for location and include a new file from the base file for the purpose.
  - this should all be on the holdings tab
  - when i change the gain timeline, everything else should update, too, notably the "value over time" which is fixed to previous 12 months
  - the gain timeline also needs more options. lets do 5yr, 3mo, 1mo, and 1 week as additions
- feat: intelligent category suggestions
  - only real way to do this is with some sort of lookback comparing similar descriptions in the past and seeing associated expense or revenue accounts
  - need to remove random numbers from description and maybe do a predominance calculation or a vector comparison rather than full equality.  if we're doing equality and removing numbers, we need to normalize some by lowercasing.  but in a perfect world, "netflix.com" might see a previous "netflix" and guess category based on that.  the more exact the match and the more recent, the higher the sort ranking
  - feat: remember categorization functionality — write a chosen category back into the rules file as a new `if` rule (the rules editor and its write path are done; this is the one-click path into them)
- feat: File -> New
  - Here I'm assuming we're setting up a new set of journal files, chart of accounts, etc. Probably we prompt with some questions and use an empty folder as a starting point and then create a skeleton so someone can start using us to track things. We should have a default chart of accounts for individuals and another for businesses and then we should allow them to start with an empty set of accounts to add their own.
- feat: saved report filters?
- feat: planning calculators a la quicken financial planner; see inspiration from [credit karma](https://www.creditkarma.com/calculators/money) and [nerdwallet](https://www.nerdwallet.com/investing/calculators)
  - great free tools with details at [engaging-data](https://engaging-data.com/early-retirement-calculators-and-tools/)
  - investigate [projection lab](https://projectionlab.com) to understand if that's worthwhile or anything there we want to learn from. from a friend: "really nice stuff built on top of it (roth conversions, drawdown simulation, flex spending, tax strategy, "what if" checkpointing to compare decisions, nice milestone tools to setup when costs are known to change and how, etc"

