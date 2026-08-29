# Releasing Ledgeline

Tagging `vX.Y.Z` builds, signs, notarizes and publishes a macOS DMG.
`.github/workflows/release.yml` is the whole pipeline; this document is the part
that cannot live in a comment — the one-time Apple setup, and what to do when
notarization says no.

## What ships

Two DMGs — `Ledgeline-X.Y.Z-arm64.dmg` and `Ledgeline-X.Y.Z-x86_64.dmg` — built
by a two-leg matrix and attached to one GitHub Release. Each contains a
`Ledgeline.app` which is:

- **Self-contained.** Every load command in both Mach-O files resolves against
  the base OS (`/usr/lib/*` and system frameworks). `flake.nix` asserts this at
  build time rather than trusting it, so a bundle that would only launch on the
  build machine fails the build instead of a user's Mac.
- **Complete for imports.** `hledger` 1.52.1 is bundled in
  `Contents/MacOS/hledger`, beside the app binary. Users install nothing else.
- **Signed and notarized**, with the ticket stapled into *both* the app and the
  DMG — so a downloaded copy opens normally, including offline.

### Two DMGs, not a universal binary

`lipo -create` over the two `ledgelineWithSpa` binaries would give one fatter
download, but it needs both slices on one runner — and the bundle is per-arch
anyway, `hledger` included. Two independent matrix legs are simpler, and each
user downloads only what they need.

The matrix is the only place the architecture is named:

```yaml
runs-on: macos-latest        # BOTH legs
matrix:
  include:
    - {arch: arm64,  nixSystem: aarch64-darwin}
    - {arch: x86_64, nixSystem: x86_64-darwin}
```

Everything downstream reads `matrix.arch` / `matrix.nixSystem`, so retiring
Intel is a one-line deletion.

### Both legs run on Apple Silicon; Intel goes through Rosetta

There is no Intel runner, and there cannot be one:

```
Error: Determinate Nix Installer no longer supports macOS on Intel.
Please migrate to Apple Silicon, and use Nix's built-in Rosetta support
to build for Intel.
```

`macos-15-intel` still exists as a GitHub runner, but nothing can install Nix
on it, and this pipeline is `nix build` end to end. So the x86_64 leg runs on
`macos-latest` with `extra-platforms = x86_64-darwin` in `nix.conf` and Rosetta
2 installed, and builds `.#packages.x86_64-darwin.macApp` explicitly.

Two consequences worth internalising:

- **Name the system, never `.#macApp`.** The bare attribute resolves against
  the *runner's* system, which is now aarch64-darwin on both legs — so it would
  quietly produce a second arm64 bundle, sign it, notarize it, and publish it
  as `-x86_64.dmg`. The job asserts `lipo -archs` on both Mach-Os for exactly
  this reason; that assertion is the thing standing between you and a DMG that
  cannot launch for the users it is named after.
- **The x86_64 leg is slow.** Its rustc runs emulated. Everything from nixpkgs
  substitutes as a prebuilt x86_64-darwin binary, so what is actually emulated
  is our own crates — bounded, and cached in Cachix afterwards.

This is also the arrangement that *outlives* the Intel runner: when GitHub
retires x86_64 macOS hardware, an arm64 runner can still emit an Intel DMG.

### The Intel leg is on a separate nixpkgs, and it has an expiry date

**Nixpkgs 26.11 — what `nixos-unstable` now is — dropped `x86_64-darwin`
entirely.** Not "fails to build": it refuses to *evaluate*, with

```
error: Nixpkgs 26.11 has dropped support for x86_64-darwin.
```

So the Intel leg cannot use the same nixpkgs as everything else. `flake.nix`
carries a second input, `nixpkgs-x86-darwin`, pinned to `nixpkgs-26.05-darwin`
— the last release that supports Intel Macs, security-fixed until the end of
2026 — and `hostNixpkgs` selects it for that one system. No other system sees
it.

Three dates bound this, and none of them are ours to move:

| | |
| --- | --- |
| Determinate dropped its Intel macOS installer | already done |
| Nixpkgs 26.05 security fixes end | end of 2026 |
| GitHub retires its last x86_64 macOS runner | Fall 2027 |
| Apple stopped selling Intel Macs | 2023 |

When the Intel leg goes, delete the `nixpkgs-x86-darwin` input, `hostNixpkgs`,
the `x86_64-darwin` entries in `spaNodeModulesHashes` and `hledgerAsset`, and
the matrix line. Nothing else is Intel-aware.

### The `x86_64-darwin` SPA hash starts out wrong, on purpose

`spaNodeModules` is a fixed-output derivation whose hash covers the resolved
`node_modules`, including the native `esbuild` / `rollup` /
`@tailwindcss/oxide` binaries — so it is per-platform, and **a hash can only be
generated on the platform it describes**. There is no Intel Mac here, so
`flake.nix` ships `lib.fakeHash` for `x86_64-darwin`.

The first Intel run therefore fails with a mismatch that prints the real value.
Paste it into `spaNodeModulesHashes.x86_64-darwin` and re-run. This is expected
once, and the same applies to `x86_64-linux` whenever the JS toolchain is bumped
from a Mac.

**Get that hash from a dry run, not from a tag.** The `release` job needs *both*
legs, so an Intel hash mismatch fails the whole release — after the arm64 leg
has already signed, notarized and stapled, which spends a notarization round
trip and leaves a tag with no GitHub Release. A `workflow_dispatch` run with
**dry_run** checked builds and packages both architectures and stops before
signing, so it surfaces the hash for free:

1. Actions → Release → Run workflow, `dry_run` checked.
2. The x86_64 leg fails; copy the `got: sha256-…` value out of its log.
3. Put it in `spaNodeModulesHashes.x86_64-darwin`, commit, re-run the dry run.
4. Both legs green → tag for real.

## Cutting a release

`scripts/release.sh` owns the version. Use it rather than editing files by hand:
the version lives in four places and three of them fail silently when they drift.

```sh
scripts/release.sh set 0.2.0            # rewrite every reference, verify, stop
git diff                                # review
scripts/release.sh set 0.2.0 --commit   # commit + tag (does NOT push)

git push origin main v0.2.0             # pushing the tag starts the release
```

The run takes roughly 15–25 minutes when the Cachix layer is warm, most of it
waiting on Apple: there are two notarization round trips (the app, then the DMG).

### Where the version lives, and how each one betrays you

| Location | Kept in step by | What a stale value does |
| --- | --- | --- |
| `Cargo.toml` `[workspace.package] version` | **source of truth** | — |
| `Cargo.toml` `[workspace.dependencies] ledgeline-core.version` | `release.sh set` | Nothing locally — `path` wins inside the workspace. Breaks `cargo publish`, at the end of a release |
| `Cargo.lock` | `cargo update --workspace` | Fails `--locked` builds in CI |
| `web/package.json` | `release.sh set` | Nothing at all. It had already drifted to `0.0.1` against a `0.1.0` workspace before this script existed |
| The git tag | you | Publishes `Ledgeline-0.2.0.dmg` containing an app that reports `0.1.0` in About and in every crash report forever — `Info.plist` is substituted from `Cargo.toml`, not from the tag |

`scripts/release.sh check` asserts all of them agree, and takes an optional tag
to check against. It runs in three places so drift cannot survive:

- the `versions` CI job, on every PR
- the release workflow's `package` job, against the tag
- the release workflow's `publish` job, again

```sh
scripts/release.sh current          # print the version
scripts/release.sh check            # assert every reference agrees
scripts/release.sh check v0.2.0     # ...and that the tag matches
```

### Dry runs

Run the workflow manually from the Actions tab with **dry_run** checked. It
builds and packages exactly as a real release does, then stops before signing,
notarization and publishing, and uploads the unsigned DMG as a workflow artifact.

Use it whenever the packaging changes. It is the only way to exercise the DMG
layout without spending a tag.

## One-time Apple setup

Five repository secrets. All are set under **Settings → Secrets and variables →
Actions**. You need an Apple Developer Program membership ($99/yr).

### The signing certificate

`MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD`

You want a **Developer ID Application** certificate — *not* "Mac App
Distribution", which only works for the App Store and will fail notarization for
a directly distributed app.

1. In Xcode: **Settings → Accounts → your team → Manage Certificates → + →
   Developer ID Application**. (Or create the CSR by hand at
   <https://developer.apple.com/account/resources/certificates>.)
2. In **Keychain Access**, find the certificate, expand it so both the cert and
   its private key are selected, right-click → **Export 2 items…**, save as
   `.p12`, and set a strong password. Both halves matter: a `.p12` exported
   without the private key imports fine and then fails at `codesign` with "no
   identity found".
3. Base64 it, and put the result in `MACOS_CERTIFICATE_P12`:

   ```sh
   base64 -i DeveloperID.p12 | pbcopy
   ```

4. Put the export password in `MACOS_CERTIFICATE_PASSWORD`.

The workflow derives the signing identity string from the certificate itself, so
there is no sixth secret to keep in sync.

### The notarization key

`APPLE_API_KEY_P8`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`

An App Store Connect API key, rather than an Apple ID and app-specific password:
it does not break when 2FA prompts, it is scoped, and it is revocable on its own.

1. At <https://appstoreconnect.apple.com/access/integrations/api>, create a key
   with the **Developer** role (that is sufficient for notarization).
2. Download `AuthKey_XXXXXXXXXX.p8`. **Apple lets you download it exactly once.**
3. `APPLE_API_KEY_P8` — `base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy`
4. `APPLE_API_KEY_ID` — the ten-character Key ID (the `XXXXXXXXXX` above).
5. `APPLE_API_ISSUER_ID` — the UUID shown above the key list on that page. It is
   per-team, not per-key.

## Verifying by hand

The workflow asserts all of this before publishing, but when something looks
wrong on a downloaded DMG:

```sh
# The ticket is stapled and valid offline.
xcrun stapler validate Ledgeline-0.2.0-arm64.dmg

# The string that matters is `source=Notarized Developer ID`. Anything else
# means a user gets a Gatekeeper prompt.
spctl --assess --type exec -vvv /Applications/Ledgeline.app

# Every load command resolves on a stock Mac — no /nix/store, no /opt/homebrew.
otool -L /Applications/Ledgeline.app/Contents/MacOS/ledgeline
otool -L /Applications/Ledgeline.app/Contents/MacOS/hledger
```

## When notarization fails

`notarytool submit --wait` exits non-zero and the workflow then fetches and
prints the full log, which names the offending file. The usual causes:

| Symptom in the log | Cause | Fix |
| --- | --- | --- |
| `The signature does not include a secure timestamp` | `--timestamp` missing, or Apple's timestamp server was unreachable | Re-run; it is usually transient |
| `The executable does not have the hardened runtime enabled` | A Mach-O got signed without `--options runtime` | Add it to the `sign()` helper's arguments |
| `The binary is not signed` naming `Contents/MacOS/<something>` | A new nested binary was added to the bundle but not to the sign step | Add it — the sign step lists files explicitly, on purpose |
| `Team is not yet configured for notarization` | New membership, agreements not accepted | Accept the current agreements in App Store Connect |

**Entitlements.** None are passed. The app is not sandboxed, and `wry` drives
`WKWebView`, whose JIT lives in the out-of-process `com.apple.WebKit.WebContent`
system service rather than in our address space — so `com.apple.security.cs.allow-jit`
and its relatives are not needed. If a future dependency does need one, that is
where to look first, and the entitlements plist goes in `assets/`.

## Publishing to crates.io

The `publish` job runs after the DMG job on every tag and pushes both crates, so
non-Mac users can install with:

```sh
# Headless — serves the UI over HTTP, open it in your own browser.
cargo install ledgeline --no-default-features

# With the desktop window. Needs webkitgtk-4.1, gtk3, libsoup-3 and libxdo
# (Debian/Ubuntu: libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libxdo-dev).
cargo install ledgeline
```

Two crates go up: `ledgeline` (the binary, in `crates/ledgeline-server/`) and
`ledgeline-core` (the engine it depends on). The package name and the directory
name differ on purpose — cargo does not tie them together, and `ledgeline` is
what a user types. Core has to be published even though nobody installs it
directly: `cargo publish` rejects a package whose dependency is not on the
registry, so it goes first.

### The one secret

`CARGO_REGISTRY_TOKEN` — from <https://crates.io/settings/tokens>.

**Until that secret exists the publish step skips with a notice and the job still
passes**, so tagging works today and starts publishing the day you add it. It
runs *after* the DMG job on purpose: a crates.io release cannot be withdrawn,
only yanked, so publishing before the DMG is known good would permanently attach
a version number to a release that never shipped.

Scope the CI token to **`publish-update`**, restricted to `ledgeline` and
`ledgeline-core`. That is deliberately not enough to create a crate: a token that
can `publish-new` can claim *any* unclaimed name on crates.io, and this one lives
in a CI system that runs on every tag.

### Bootstrapping the two names, once

`publish-update` cannot make the first upload — the crate has to exist first. So
the very first version of each crate is published by hand, from a machine, with a
`publish-new` token that is then deleted. Everything after that is CI's job.

This is the one release that is *not* driven by a tag, and the SPA is the trap:
`crates/ledgeline-server/spa/` is git-ignored, so a crate packaged without the
step below ships a UI that is a single line of placeholder text — at a version
number that can never be reused.

```sh
scripts/release.sh check                 # every version reference agrees

# 1. Build the real SPA and stage it where `include` will pick it up.
#    This is exactly what the workflow's "stage it into the crate" step does.
cd web && bun run build && cd ..
rm -rf crates/ledgeline-server/spa
mkdir -p crates/ledgeline-server/spa
cp -R web/build/. crates/ledgeline-server/spa/

# 2. Prove it is the real UI, not build.rs's placeholder.
grep -q "SPA not built" crates/ledgeline-server/spa/index.html \
  && { echo "PLACEHOLDER — do not publish"; exit 1; }

# 3. Publish. Core first; cargo has waited for its own upload to appear in the
#    index since 1.66, so the second command does not race the first.
#    `--allow-dirty` because `spa/` is git-ignored by design.
export CARGO_REGISTRY_TOKEN=<a publish-new token>
cargo publish -p ledgeline-core --allow-dirty
cargo publish -p ledgeline --allow-dirty
```

Then **revoke the `publish-new` token** at
<https://crates.io/settings/tokens> and add the `publish-update` one to Actions.

Note what this costs: the version you publish by hand is spent. If you hand-publish
`0.1.0`, the next *tagged* release must be `0.2.0` — a tag at `v0.1.0` would reach
`cargo publish` and fail with "crate version already uploaded", after the DMG had
already been built and released. Either hand-publish the current version and move
the tag up, or hand-publish and tag the same version knowing the publish job will
fail loudly on that one run.

### What makes the crate publishable

Worth knowing, because it is easy to undo by accident:

- **`spa.rs` embeds `$CARGO_MANIFEST_DIR/spa`, not `../../web/build`.**
  `cargo package` refuses to include files outside the package root, so the old
  path made the crate unpublishable — it would have shipped with no UI.
- **`build.rs` has three cases** (workspace / published crate / neither). The
  published-crate case must never overwrite the shipped `spa/`, or every
  `cargo install` serves the placeholder.
- **`spa/` is git-ignored** (it is a build artifact) but named in
  `ledgeline`'s `include` list, which overrides `.gitignore` at packaging
  time.
- **The workflow asserts the staged SPA is not the placeholder** before
  publishing. A placeholder packages exactly as happily as the real thing — same
  filename, same manifest, no warning — and the result would be an unyankable
  version whose entire UI is an error page.
- **`ledgeline-core` carries a `version` in the workspace dependency table.**
  `cargo publish` rejects a path dependency without one.

Rehearse the whole thing locally without publishing anything:

```sh
cd web && bun run build && cd ..     # populate web/build so build.rs mirrors it
cargo package --workspace --allow-dirty
tar tzf target/package/ledgeline-[0-9]*.crate | grep -c '/spa/'   # expect ~76
```

## Bumping the bundled hledger

`flake.nix` pins the version and the per-architecture tarball hashes in
`bundledHledger`. To move to a new release:

```sh
nix store prefetch-file --hash-type sha256 \
  https://github.com/simonmichael/hledger/releases/download/1.53/hledger-mac-arm64.tar.gz
nix store prefetch-file --hash-type sha256 \
  https://github.com/simonmichael/hledger/releases/download/1.53/hledger-mac-x64.tar.gz
```

Update `bundledHledgerVersion` and both hashes, then `nix build .#macApp`. The
derivation asserts the new binary links only system libraries, so a change in how
hledger builds its releases fails there rather than on a user's Mac.

Keep it at or above `MIN_HLEDGER` in `crates/ledgeline-server/src/hledger.rs`
(currently 1.40, the release that renamed `--rules-file` to `--rules`).

## How the app finds hledger

Resolution order lives in `crates/ledgeline-server/src/hledger.rs`. For a
downloaded release only one step matters, and it is worth understanding why:

1. `prefs.hledger_path` — the settings form.
2. `$LEDGELINE_HLEDGER`.
3. The baked `LEDGELINE_HLEDGER_PATH` — a `/nix/store/…` path. Correct on the
   build machine, **absent** on a user's Mac, where the stat check drops it.
4. **A sibling `hledger` next to our own executable.** This is the one the DMG
   relies on.
5. `hledger` on `$PATH`.

Step 5 is not the safety net it looks like. A process launched from Finder or the
Dock inherits *launchd's* environment, not the one your shell exports from
`.zshrc` — so a user with Homebrew hledger working perfectly in their terminal
still gets "hledger was not found" from a double-clicked app. That is why the
bundle ships its own copy, and why step 4 exists.
