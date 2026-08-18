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
- **Holdings** — average-cost basis, unrealized gain (all-time / year-to-date / trailing-12-months),
  value-over-time, per-symbol names from `commodity` directives, partial portfolio totals; XLSX export.
- **Imports** — finds the CSV import rules files (`*.rules`) beside your journal and edits them in a
  friendly form instead of a text box: date format, the CSV column → field mapping (labelled with your
  data file's real headers and sample values), the default accounts, and a reorderable list of `if`
  rules. Anything fancier than a plain OR rule — `if` tables, `&&`/`!` matchers, match groups — is shown
  read-only rather than rewritten, and saving preserves the rest of the file byte for byte.
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
# then:  ledgeline ~/finance/2026.journal        # or launch Ledgeline.app
```

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

## Architecture

This spins up a local tokio axum API server and uses the native OS browser as a GUI window (via wry, part of the tauri project) hosting a svelte frontend app.  All assets are built into the single binary, which is pretty snappy.

## TODO

- feat: import drag/drop
  - command line options
  - fix styling of numbers issues; infected the entire ui now
- feat: create new import rules
  - take a csv file and make intelligent guesses on setup. we want intelligent mapping of headings, ask what account it is and default categorizations, figure out ordering of rows. detect separator, skip rows number, and encoding automatically. figure out date-format automatically. 
- feat: edit budget
  - figure out where budget rules already exist and that's where we'll store new lines and update existing ones
- Better keyboard navigation
  - tab complete account selection
  - enter to save transaction edit
- chore: Setup releases and builds for download
- chore: Add screenshots and better descriptions to the readme
- feat: zillow integration
  - Need a way to map an asset to an address. Maybe a special comment in the accounts file?
  - Need a way to map the unrealized gains for that address
  - Then on some sort of "update" click (how/where on UI?), it fetches the latest value (or launches a page and then prompts for it?), calculates the difference relative to the current asset value and then makes an adjustment to the unrealized account with a comment saying the current zestimate
- rules edit ui improvements
  - we need to figure out a new rules editor approach because the current one is ugly, hard to find what you're looking for, very long vertically and not scannable
  - also: we can't do more sophisticated rules (with conditional logic in them) so we need to add that and figure out ways to display and edit them
  - perhaps instead of one giant form, we have display separate from edit and can therefore make this nicer
- feat: private AI integration?
- feat: stock price updates
  - basically my script, maybe ported into rust, for querying yahoo and updating a prices file. should try to figure out where prices already live and if it can't find anything, prompt for location and include a new file from the base file for the purpose.
  - this should all be on the holdings tab
  - when i change the gain timeline, everything else should update, too, notably the "value over time" which is fixed to previous 12 months
  - the gain timeline also needs more options. lets do 5yr, 3mo, 1mo, and 1 week as additions
- feat: intelligent category suggestions
  - only real way to do this is with some sort of lookback comparing similar descriptions in the past and seeing associated expense or revenue accounts
  - need to remove random numbers from description and maybe do a predominance calculation or a vector comparison rather than full equality.  if we're doing equality and removing numbers, we need to normalize some by lowercasing.  but in a perfect world, "netflix.com" might see a previous "netflix" and guess category based on that.  the more exact the match and the more recent, the higher the sort ranking
  - feat: remember categorization functionality — write a chosen category back into the rules file as a new `if` rule (the rules editor and its write path are done; this is the one-click path into them)
- feat: saved report filters?
- feat: planning calculators a la quicken financial planner; see inspiration from [credit karma](https://www.creditkarma.com/calculators/money) and [nerdwallet](https://www.nerdwallet.com/investing/calculators)
  - great free tools with details at [engaging-data](https://engaging-data.com/early-retirement-calculators-and-tools/)
  - investigate [projection lab](https://projectionlab.com) to understand if that's worthwhile or anything there we want to learn from. from a friend: "really nice stuff built on top of it (roth conversions, drawdown simulation, flex spending, tax strategy, "what if" checkpointing to compare decisions, nice milestone tools to setup when costs are known to change and how, etc"
