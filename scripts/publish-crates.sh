#!/usr/bin/env bash
# Publish ledgeline-core, ledgeline-shell and ledgeline to crates.io.
#
#   scripts/publish-crates.sh --dry-run   package and verify-build all three
#   scripts/publish-crates.sh             ...and upload them
#
# Used by release.yml for both: the `crates-dry-run` job (every run, dry or
# tagged) and the `publish` job (tags only, after the GitHub Release). One
# script, so the rehearsal is the same command as the real thing.
#
# # Why one `cargo publish` and not three
#
# Since cargo 1.90, `cargo publish -p a -p b -p c` packages every selected
# crate and verify-builds each against a local overlay of the others BEFORE
# uploading any of them. Three separate calls uploaded core and shell first,
# so a `ledgeline` that failed its verify build left them on the registry
# (yank-only, never deletable) while `cargo install ledgeline` still gave the
# previous version. Uploads go in dependency order: core, shell, ledgeline.
#
# # The SPA
#
# `crates/ledgeline-server/spa/` is git-ignored, so a fresh checkout has none,
# and `build.rs` would write its placeholder there for cargo to package. The
# real SPA is built by Nix (`.#spaBuild`) and staged in, then checked:
# a placeholder packages exactly as happily as the real UI, and the result
# would be a published, unyankable version whose UI is one line of text.
#
# `--allow-dirty` is load-bearing: `spa/` was just written and is git-ignored
# by design, so cargo would otherwise refuse to package it. Run from a clean
# checkout (CI's is a tag) so nothing else can be dirty.
#
# The invoicer's crates (`invoicer-core`, `ledgeline-invoicer`) and
# `ledgeline-ffi` are `publish = false` and are not selected here (invoicer
# PLAN I19).
set -euo pipefail

cd "$(dirname "$0")/.."

# A plain string, not an array: macOS's /bin/bash 3.2 treats an empty array
# under `set -u` as unbound.
mode=
case "${1:-}" in
  --dry-run) mode=--dry-run ;;
  "") ;;
  *) echo "usage: $0 [--dry-run]" >&2; exit 2 ;;
esac

spa=crates/ledgeline-server/spa

echo "==> Building the SPA and staging it into $spa"
nix build -L .#spaBuild -o result-spa
rm -rf "$spa"
mkdir -p "$spa"
cp -RL result-spa/. "$spa/"
chmod -R u+w "$spa"

index=$spa/index.html
if [ ! -f "$index" ]; then
  echo "error: $index is missing" >&2
  exit 1
fi
if grep -q "SPA not built" "$index"; then
  echo "error: the staged SPA is the build.rs PLACEHOLDER, not the real UI." >&2
  echo "  Publishing this would ship a crate whose entire UI is an error page." >&2
  exit 1
fi
echo "Staged SPA looks real ($(find "$spa" -type f | wc -l | tr -d ' ') files)."

echo "==> cargo publish $mode"
# shellcheck disable=SC2086 # $mode is empty or one word
nix develop path:. --command cargo publish $mode \
  -p ledgeline-core -p ledgeline-shell -p ledgeline --allow-dirty
