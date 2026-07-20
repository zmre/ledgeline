# Ledgeline development

Ledgeline is a single-binary app: `ledgeline` = an axum HTTP server + a
wry/tao desktop webview (default-on `gui` cargo feature), with the built
SvelteKit SPA (`web/build`) embedded into the binary via `rust-embed`. Build
`--no-default-features` for a headless, server-only binary (the shape Linux
servers and CI build).

The whole toolchain is provided by Nix. Do not install anything globally.

## Dev shell

```sh
direnv allow                 # auto-loads the flake dev shell (.envrc)
# or, without direnv:
nix develop path:.
```

The dev shell provides the pinned Rust toolchain, `pkg-config`, `nodejs_22`,
`bun`, `hledger`, `hledger-web`, `just`, and the Playwright browsers, plus the
Linux GUI libraries (`webkitgtk_4_1`, `gtk3`, `libsoup_3`). It also exports
`LEDGELINE_FIXTURE` and the Playwright env vars. Common tasks live in the
`justfile` (`just --list`).

```sh
nix develop path:. --command bash -c 'cargo --version && node --version && bun --version && hledger --version'
```

## The crane caching model (why rebuilds/retests are fast)

`flake.nix` uses [crane](https://github.com/ipetkov/crane). The key idea:

1. `cargoArtifacts = craneLib.buildDepsOnly …` compiles **only the third-party
   dependencies** (including the entire wry/tao GUI stack) from a dummy source.
   This is a single cached layer in the Nix store.
2. Every real output — the binary and each check — is built with
   `… // { inherit cargoArtifacts; }`, so they **reuse** that layer instead of
   recompiling dependencies.

Because the dependency layer's hash depends only on `Cargo.toml` / `Cargo.lock`
(not on your source), editing a `.rs` file or a fixture recompiles only the
workspace crates — the dependency layer is fetched from the store unchanged.
The first build populates the layer (minutes); subsequent builds/tests are much
faster. Push that layer to Cachix (below) and CI + teammates skip it entirely.

### Outputs

| Command | What it does |
| --- | --- |
| `nix build .#ledgeline` | The `ledgeline` binary (proves the GUI deps link: webkitgtk on Linux, system WebKit on macOS). It is `.#default` on Linux. |
| `nix build .#clippy` | `cargo clippy --all-targets -- -D warnings` |
| `nix build .#tests` | `cargo test` over the whole workspace |
| `nix build .#fmt` | `cargo fmt --check` |
| `nix build .#macApp` | **macOS only** — just the `result/Applications/Ledgeline.app` bundle, **real** SPA embedded (see below). |
| `nix build` (bare, macOS) | **macOS only** `.#default` — the combined `result/{bin/ledgeline, Applications/Ledgeline.app}` (CLI on PATH + the app). |
| `nix flake check` | Runs all of the checks above |
| `nix run .` | **Build and run the real app.** On macOS runs `ledgelineWithSpa` (the **real** SPA embedded); on Linux runs the placeholder-SPA `ledgeline` (see below). |

The bare attribute (`.#clippy`, `.#tests`, …) resolves to the current system
automatically (`x86_64-linux`, `aarch64-darwin`, …), which is how CI invokes
them on each runner.

### `nix run` runs the real app (darwin)

`apps.default` — what `nix run .` and `nix run github:zmre/ledgeline` execute —
resolves per platform:

- **macOS** → `ledgelineWithSpa`, the binary with the actual SvelteKit UI baked
  in. So `nix run github:zmre/ledgeline -- ~/finance/2026.journal` builds the SPA
  (the `bun install` FOD → `vite build`) and opens the real desktop window on
  that journal. (`.#ledgeline` still embeds the CI placeholder SPA; `apps.default`
  deliberately does **not** use it on darwin.)
- **Linux** → `ledgeline`, the placeholder-SPA binary. The real-SPA path pulls the
  `spaNodeModules` fixed-output derivation, whose `outputHash` is pinned
  per-platform (aarch64-darwin today); the Linux hash can only be produced by
  building on Linux. Nix laziness keeps `ledgelineWithSpa` from ever being forced
  on Linux, so the darwin-only FOD hash never trips a Linux eval/build.

Only `apps.default` is platform-conditional here — `packages.default`,
`.#ledgeline`, the `checks`, and `.#macApp` are unchanged.

**Follow-up — real-SPA `nix run` on Linux.** Promote `spaNodeModules` /
`spaBuild` / `ledgelineWithSpa` (and the currently darwin-guarded `packages`) to
all systems with a **per-system** `outputHash` — the Linux hash pinned from a
Linux/CI build (build once with a fake hash; Nix prints the real one), the way
`spaNodeModules.outputHash` is pinned for aarch64-darwin today. Until then,
`nix build .#ledgeline` gives the working **headless** binary on Linux
(`./result/bin/ledgeline --server`).

## The embedded SPA and the Nix sandbox

`crates/ledgeline-server/src/spa.rs` embeds `web/build` at compile time:

```rust
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/build"]
struct SpaAssets;
```

`web/build` is a git-ignored SvelteKit build artifact and is **absent** in the
Nix sandbox — and rust-embed refuses to *compile* if the folder is missing.
**We do not build the SvelteKit SPA inside Nix** (out of scope). Instead, the
flake's `preBuild` drops a placeholder `web/build/index.html` before every cargo
phase, so `buildPackage` / `cargoClippy` / `cargoTest` compile cleanly. (This
mirrors what `crates/ledgeline-server/build.rs` already does for a bare
`cargo build`, but the flake does it too so the crane sandbox never depends on
the build script's filesystem writes.)

Consequence: **Nix/CI binaries embed a placeholder SPA.** That is intentional —
CI's job is to prove the Rust compiles/links and the tests pass. The real
single binary, with the actual UI baked in, is produced with the SPA built
first (next section).

> The crane source filter also keeps the repo `fixtures/` tree in the build
> source, because the integration tests read it at runtime
> (`CARGO_MANIFEST_DIR/../../fixtures`). `web/build` is deliberately excluded so
> the Nix build stays reproducible (always the placeholder, never a stale local
> SPA).

## Building the real single binary (SPA embedded)

Order matters — build the SPA first, then the Rust binary embeds it:

```sh
cd web && bun run build        # writes web/build/
cd .. && cargo build --release # embeds web/build/ into target/release/ledgeline
```

(Or run both from the dev shell.) A plain `cargo build` without a prior
`bun run build` still works — it just embeds the placeholder shell.

## The macOS app bundle (`Ledgeline.app`)

`nix build .#macApp` (macOS only) produces `result/Applications/Ledgeline.app`
— the standard nix-darwin app layout, so it can be dragged to `/Applications`
or picked up by home-manager / nix-darwin. The bundle holds
`Contents/MacOS/ledgeline` (the binary), `Contents/Info.plist` (version taken
from the workspace `Cargo.toml`), and `Contents/Resources/ledgeline.icns`
(generated from `assets/ledgeline.png`).

On macOS the **default** package — a bare `nix build` (or `nix profile install`)
— is the combined `result/{bin/ledgeline, Applications/Ledgeline.app}`, so an
install puts the CLI on `PATH` (via `bin/`) AND the app where nix-darwin /
home-manager link it (via `Applications/`); both use the same real-SPA binary.
On Linux the default is the headless `ledgeline` binary.

`just package-mac` wraps `.#macApp` and copies a writable copy to `dist/Ledgeline.app`.

Unlike `.#ledgeline` (which embeds the CI placeholder SPA), **`.#macApp` embeds
the real SvelteKit UI**: the flake builds the SPA inside Nix — a fixed-output
`bun install` derivation (`spaNodeModules`, its hash pinned from `web/bun.lock`)
feeds an offline `vite build` (`spaBuild`), whose output is baked into a
dedicated crane build of the binary. No prior `bun run build` is required. If
`web/bun.lock` changes, re-pin `spaNodeModules.outputHash` (build once with a
fake hash; Nix prints the real one). The pinned hash captures the host
platform's native deps (esbuild/rollup/@tailwindcss/oxide), so it is
`aarch64-darwin`-specific.

The icon is assembled with `imagemagick` + `png2icns` (libicns) — no macOS
`iconutil`, so it builds in the pure Nix sandbox. The bundled binary still links
Nix-store dylibs; producing a signed, relocatable release (`install_name_tool` +
`codesign`) is a follow-up.

## Cachix binary cache

The flake declares three substituters in `nixConfig`: `cache.nixos.org`,
`nix-community.cachix.org` (both public — immediate pull benefit), and
`zmre.cachix.org` — the shared cache we reuse from
[zmre/mbr-markdown-browser](https://github.com/zmre/mbr-markdown-browser). Its
real public key is already committed in `flake.nix`, and `.envrc` passes
`--accept-flake-config` so the dev shell trusts all three. **Pulls need no
setup** — everyone (and every fork) benefits immediately.

### One-time setup the maintainer must do by hand

Only one thing is required, and only to enable CI *pushes*:

- Add the `zmre` cache's push auth token to GitHub as a repository secret named
  `CACHIX_AUTH_TOKEN` (repo **Settings → Secrets and variables → Actions → New
  repository secret**). This is the same token used by mbr.

Until that secret is set (and on forks), CI still runs fine: the cache **pushes
are skipped** when the secret is absent; **pulls** from all three caches still
work.

Push a build manually:

```sh
nix build --json .#ledgeline | jq -r '.[].outputs | to_entries[].value' | cachix push zmre
```

## Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and on every PR.
Each job installs Nix with `DeterminateSystems/determinate-nix-action@v3` and
wires `cachix/cachix-action@v17` (cache `zmre`, pulling `nix-community`
too). Jobs:

| Job | Runner(s) | Command |
| --- | --- | --- |
| `format-check` | ubuntu | `nix build -L .#fmt` (runs first) |
| `clippy` | ubuntu + macos | `nix build -L .#clippy` |
| `tests` | ubuntu + macos | `nix build -L .#tests` |
| `build` | ubuntu + macos | `nix build -L .#ledgeline` (+ pushes to Cachix on `main`) |
| `spa` | ubuntu | `bun install --frozen-lockfile && bun run build && bun run test:unit && bun run check` |

`clippy`, `tests`, and `build` depend on `format-check` and share the crane
dependency layer (populated once, then pulled from Cachix on later runs).
Playwright e2e is not part of this workflow yet.
