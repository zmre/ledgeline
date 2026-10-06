#!/usr/bin/env bash
# Fail if web/bun.lock changed on this branch but the spaNodeModules hashes in
# flake.nix did not.
#
# Why a diff heuristic rather than a build: a stale FOD pin is SILENT. The
# store path comes from the hash, so while the old path is still in a local
# store or on Cachix, Nix substitutes it and never runs `bun install` to notice
# (docs/development.md, "A stale pin is invisible until the cache drops the
# path"). And a Mac cannot build the x86_64-linux tree at all, so the `Build
# (ubuntu-latest)` job is the first thing that can see it — twenty minutes
# after the push. Comparing against the merge base is instant and catches the
# common case: a bun.lock change with no re-pin alongside it.
#
# Fix with `just repin-spa-hashes`.
#
# Base ref is origin/main; override with LEDGELINE_BASE_REF.
set -euo pipefail

cd "$(dirname "$0")/.."

base_ref=${LEDGELINE_BASE_REF:-origin/main}
base=$(git merge-base HEAD "$base_ref")

# Working tree vs merge base, so uncommitted edits count too when this runs by
# hand. At push time the tree normally matches HEAD.
if git diff --quiet "$base" -- web/bun.lock; then
  exit 0
fi

# x86_64-darwin is left out on purpose: only a release dry run under Rosetta
# can produce it (docs/releasing.md), so it cannot be required per-push.
# Reads flake.nix on stdin; only looks inside the spaNodeModulesHashes block,
# since other per-system hash maps (hledgerAsset) use the same keys.
pin() {
  sed -n '/spaNodeModulesHashes = {/,/};/p' |
    grep -E "^\s*$1 = \"sha256-" | sed -E 's/.*"(sha256-[^"]+)".*/\1/'
}

stale=()
for system in aarch64-darwin x86_64-linux; do
  old=$(git show "$base:flake.nix" | pin "$system")
  new=$(pin "$system" < flake.nix)
  if [ "$old" = "$new" ]; then
    stale+=("$system")
  fi
done

if [ "${#stale[@]}" -eq 0 ]; then
  exit 0
fi

cat >&2 <<EOF
web/bun.lock changed since $base_ref, but spaNodeModulesHashes in flake.nix
still has the old pin for: ${stale[*]}

Every bun.lock change alters the resolved node_modules and so its hash. CI's
Build (ubuntu-latest) will fail with a hash mismatch. Re-pin with:

  just repin-spa-hashes

If this bun.lock change really leaves node_modules byte-identical, check with
\`nix build .#spaNodeModules --rebuild\` and push with --no-verify.
EOF
exit 1
