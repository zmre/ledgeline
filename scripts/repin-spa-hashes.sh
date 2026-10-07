#!/usr/bin/env bash
# Re-pin spaNodeModulesHashes in flake.nix after a web/bun.lock change.
#
# For each system: swap in a placeholder hash, build the fixed-output
# derivation, and paste in the `got:` hash Nix reports. This is the manual
# routine from docs/development.md, scripted.
#
#   aarch64-darwin / x86_64-linux  built locally when that is this machine.
#   x86_64-linux from a Mac        built on a remote Linux store, by default
#                                  ssh-ng://avalon (LEDGELINE_LINUX_STORE to
#                                  override, or set it empty to skip). This
#                                  goes through `--store` rather than the
#                                  daemon's /etc/nix/machines builders: the
#                                  daemon runs as root and may not have the SSH
#                                  key, but `--store` uses yours.
#   x86_64-darwin                  never: release dry run only (docs/releasing.md).
#
# A system that cannot be built keeps its current pin, and is reported.
set -euo pipefail

cd "$(dirname "$0")/.."

linux_store=${LEDGELINE_LINUX_STORE-ssh-ng://avalon}
placeholder="sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
current=$(nix eval --impure --raw --expr builtins.currentSystem)

# Both helpers only touch the spaNodeModulesHashes block: other per-system
# hash maps in flake.nix (hledgerAsset) use the same keys.
current_pin() {
  sed -n '/spaNodeModulesHashes = {/,/};/p' flake.nix |
    grep -E "^\s*$1 = \"sha256-" | sed -E 's/.*"(sha256-[^"]+)".*/\1/'
}

set_pin() {
  # perl rather than sed -i: GNU and BSD sed disagree on -i.
  SYSTEM="$1" HASH="$2" perl -pi -e \
    'if (/spaNodeModulesHashes = \{/ .. /\};/) { s/^(\s*\Q$ENV{SYSTEM}\E = ")sha256-[^"]+(";)/$1$ENV{HASH}$2/ }' flake.nix
}

# Build the FOD for $1 with the placeholder in place; print the real hash.
real_hash() {
  local system=$1 out
  if [ "$system" = "$current" ]; then
    out=$(nix build ".#packages.$system.spaNodeModules" --no-link 2>&1 || true)
  else
    local drv
    drv=$(nix eval --raw ".#packages.$system.spaNodeModules.drvPath")
    out=$(nix build --eval-store auto --store "$linux_store" "$drv^out" --no-link 2>&1 || true)
  fi
  # Strip ANSI colour codes before matching.
  printf '%s\n' "$out" | sed -E 's/\x1b\[[0-9;?]*[a-zA-Z]//g' |
    sed -nE 's/^[[:space:]]*got:[[:space:]]+(sha256-[^[:space:]]+).*/\1/p' | head -1
}

skipped=()
for system in aarch64-darwin x86_64-linux; do
  if [ "$system" != "$current" ] && { [ "$system" != x86_64-linux ] || [ -z "$linux_store" ]; }; then
    skipped+=("$system")
    continue
  fi
  old=$(current_pin "$system")
  echo "==> $system (currently $old)"
  set_pin "$system" "$placeholder"
  new=$(real_hash "$system")
  if [ -z "$new" ]; then
    set_pin "$system" "$old"
    echo "    could not get a hash; pin left unchanged" >&2
    skipped+=("$system")
    continue
  fi
  set_pin "$system" "$new"
  if [ "$new" = "$old" ]; then
    echo "    unchanged"
  else
    echo "    re-pinned to $new"
  fi
done

if [ "${#skipped[@]}" -gt 0 ]; then
  echo "NOT re-pinned: ${skipped[*]}. Build there, or let CI report the hash." >&2
  exit 1
fi
