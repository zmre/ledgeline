#!/usr/bin/env bash
# The single owner of "what version is Ledgeline".
#
#   scripts/release.sh current            print the version
#   scripts/release.sh check [vX.Y.Z]     assert every reference agrees
#                                         (and, given a tag, that it matches)
#   scripts/release.sh set X.Y.Z          rewrite every reference
#   scripts/release.sh set X.Y.Z --commit ...and commit + tag it
#
# # Why this exists
#
# The version is not stored once. `[workspace.package] version` is the source of
# truth, but three other places have to agree with it, and each drifts silently
# in its own way:
#
#   * `[workspace.dependencies] ledgeline-core.version`,
#     `ledgeline-shell.version` and `invoicer-core.version` — REQUIRED for
#     `cargo publish` (a path dependency with no version is rejected), and cargo
#     has no `version.workspace` inheritance for it. A stale value here does not
#     break any local build, because `path` wins inside the workspace. It breaks
#     the publish, at the end of a release, after the DMG is already out.
#   * `Cargo.lock` — derived, but only if something re-resolves it. A bumped
#     Cargo.toml with a stale lock makes `--locked` builds fail in CI.
#   * `web/package.json` and `invoicer/web/package.json` — invisible to the
#     Rust build entirely, which is why web's was already sitting at 0.0.1
#     against a workspace at 0.1.0 when this script was written.
#   * `bun.lock` — the bun workspace's lockfile records each member's version,
#     both SPAs' included. `bun install --frozen-lockfile` tolerates a stale one, but
#     the next plain `bun install` rewrites it, so it is kept in step here.
#     (scripts/check-spa-hash.sh knows that such a diff changes no node_modules.)
#
# `check` is wired into CI so drift cannot land, and into the release workflow so
# a tag cannot ship against a tree that disagrees with it.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

CARGO_TOML="Cargo.toml"
# The SPAs' manifests, and their bun.lock workspace keys (their paths).
PACKAGE_JSONS=("web/package.json" "invoicer/web/package.json")
BUN_LOCK_MEMBERS=("web" "invoicer/web")
BUN_LOCK="bun.lock"
# The workspace crates whose versioned [workspace.dependencies] entry
# (`name = { path = "...", version = "..." }`) must follow the workspace, and
# every workspace crate Cargo.lock records.
DEP_CRATES=("ledgeline-core:crates/ledgeline-core" "ledgeline-shell:crates/ledgeline-shell" "invoicer-core:invoicer/crates/invoicer-core")
LOCK_CRATES=("ledgeline-core" "ledgeline-shell" "ledgeline" "invoicer-core" "ledgeline-invoicer")

die() { echo "error: $*" >&2; exit 1; }

# --- readers -----------------------------------------------------------------
# Each reader prints one version, or nothing if the line it owns has gone
# missing. A missing value is reported as a mismatch rather than crashing, so
# `check` explains what happened instead of dying on an empty variable.

# `^version = "..."` matches ONLY the `[workspace.package]` entry: every other
# version in this file is an inline table value (`serde = { version = "1" }`) or
# a bare string (`ropey = "1.6"`), neither of which starts a line with `version`.
read_workspace_version() {
    sed -n -E 's/^version = "([^"]+)"$/\1/p' "$CARGO_TOML" | head -1
}

# `$1` is a DEP_CRATES entry, `name:path`.
read_dep_version() {
    local name="${1%%:*}" path="${1#*:}"
    NAME="$name" DEP_PATH="$path" perl -ne \
        'if (/^\Q$ENV{NAME}\E = \{ path = "\Q$ENV{DEP_PATH}\E", version = "([^"]+)" \}$/) { print "$1\n"; exit }' \
        "$CARGO_TOML"
}

# The FIRST `"version":` in package.json is the package's own; anything later
# would be inside a nested object. `head -1` pins that.
read_package_json_version() {
    sed -n -E 's/^[[:space:]]*"version": "([^"]+)",?$/\1/p' "$1" | head -1
}

# bun.lock's `"web": { "name": "web", "version": "…" }` workspace entry (keyed by
# the member's path, `$1`): the first `"version":` after its opening line.
read_bun_lock_version() {
    awk -v open="    \"$1\": {" '
        $0 == open { found = 1; next }
        found && /"version": / { gsub(/[",]/, "", $2); print $2; exit }
        found && /^    \}/ { exit }
    ' "$BUN_LOCK"
}

# Cargo.lock records every workspace member. They must agree with each other and
# with the workspace version; printing them separately lets `check` say which.
read_lock_version() {
    awk -v crate="$1" '
        $0 == "name = \"" crate "\"" { found = 1; next }
        found && /^version = / { gsub(/[",]/, "", $3); print $3; exit }
    ' Cargo.lock
}

# --- commands ----------------------------------------------------------------

cmd_current() {
    local version
    version=$(read_workspace_version)
    [ -n "$version" ] || die "could not read [workspace.package] version from $CARGO_TOML"
    printf '%s\n' "$version"
}

cmd_check() {
    local expected_tag="${1:-}"
    local version failures=0
    version=$(read_workspace_version)
    [ -n "$version" ] || die "could not read [workspace.package] version from $CARGO_TOML"

    echo "workspace version: $version"

    check_one() {
        local label="$1" actual="$2"
        if [ -z "$actual" ]; then
            echo "  MISSING  $label — could not find the version line at all" >&2
            failures=$((failures + 1))
        elif [ "$actual" != "$version" ]; then
            echo "  DRIFT    $label = $actual (expected $version)" >&2
            failures=$((failures + 1))
        else
            echo "  ok       $label = $actual"
        fi
    }

    local dep pkg member crate
    for dep in "${DEP_CRATES[@]}"; do
        check_one "Cargo.toml [workspace.dependencies] ${dep%%:*}" "$(read_dep_version "$dep")"
    done
    for pkg in "${PACKAGE_JSONS[@]}"; do
        check_one "$pkg" "$(read_package_json_version "$pkg")"
    done
    for member in "${BUN_LOCK_MEMBERS[@]}"; do
        check_one "bun.lock $member" "$(read_bun_lock_version "$member")"
    done
    for crate in "${LOCK_CRATES[@]}"; do
        check_one "Cargo.lock $crate" "$(read_lock_version "$crate")"
    done

    if [ -n "$expected_tag" ]; then
        if [ "$expected_tag" != "v$version" ]; then
            echo "  DRIFT    git tag $expected_tag (expected v$version)" >&2
            failures=$((failures + 1))
        else
            echo "  ok       git tag $expected_tag"
        fi
    fi

    if [ "$failures" -gt 0 ]; then
        echo >&2
        echo "$failures version reference(s) disagree with $CARGO_TOML." >&2
        echo "Run: scripts/release.sh set $version" >&2
        exit 1
    fi
    echo "All version references agree."
}

cmd_set() {
    local version="${1:-}" commit="${2:-}"
    [ -n "$version" ] || die "usage: release.sh set X.Y.Z [--commit]"

    # Semver, and deliberately strict: a tag like `v1.2` or `1.2.3-rc.1 ` would
    # sail through a looser pattern and produce an Info.plist Apple rejects.
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
        || die "'$version' is not a semver version (expected X.Y.Z or X.Y.Z-pre)"

    if [ -n "$commit" ] && [ "$commit" != "--commit" ]; then
        die "unknown argument '$commit' (did you mean --commit?)"
    fi

    # Only enforced for --commit: rewriting files in a dirty tree is a normal
    # thing to do while preparing a release, but committing one is not.
    if [ "$commit" = "--commit" ] && [ -n "$(git status --porcelain)" ]; then
        die "working tree is dirty; commit or stash before 'set $version --commit'"
    fi

    local previous
    previous=$(read_workspace_version)
    echo "Bumping $previous → $version"

    # `sed -i` is not portable (GNU wants no argument, BSD/macOS wants one), and
    # this repo is developed on macOS and built on Linux. perl -i behaves the
    # same on both.
    perl -pi -e "s/^version = \"\Q$previous\E\"\$/version = \"$version\"/" "$CARGO_TOML"
    local dep pkg member
    for dep in "${DEP_CRATES[@]}"; do
        NAME="${dep%%:*}" DEP_PATH="${dep#*:}" VERSION="$version" perl -pi -e \
            's/^(\Q$ENV{NAME}\E = \{ path = "\Q$ENV{DEP_PATH}\E", version = )"[^"]*"( \})$/${1}"$ENV{VERSION}"${2}/' \
            "$CARGO_TOML"
    done
    # `!$done` so only the package's own version is touched, never a
    # `"version"` key nested deeper in the file.
    for pkg in "${PACKAGE_JSONS[@]}"; do
        perl -pi -e 'BEGIN { our $done = 0 } if (!$done && s/^(\s*"version": ")[^"]*(",?)$/${1}'"$version"'${2}/) { $done = 1 }' "$pkg"
    done
    # Only inside each member's workspace entry (see read_bun_lock_version).
    for member in "${BUN_LOCK_MEMBERS[@]}"; do
        MEMBER="$member" perl -pi -e 'our $in; $in = 1 if /^    "\Q$ENV{MEMBER}\E": \{$/; $in = 0 if $in && /^    \}/; s/^(\s*"version": ")[^"]*(",)$/${1}'"$version"'${2}/ if $in' "$BUN_LOCK"
    done

    # Re-resolve so Cargo.lock's own ledgeline-* entries follow. `--workspace`
    # touches only the local members, so this never silently bumps a third-party
    # dependency as a side effect of a version bump — which is exactly the kind
    # of thing that turns a release into a debugging session.
    echo "Refreshing Cargo.lock…"
    cargo update --workspace --offline --quiet \
        || cargo update --workspace --quiet \
        || die "cargo update failed; is the dev shell active? (nix develop path:.)"

    echo
    cmd_check

    if [ "$commit" = "--commit" ]; then
        echo
        git add "$CARGO_TOML" Cargo.lock "${PACKAGE_JSONS[@]}" "$BUN_LOCK"
        git commit -m "Release $version"
        git tag "v$version"
        echo
        echo "Committed and tagged v$version. Nothing has been pushed."
        echo "Push it — which is what starts the release build — with:"
        echo "    git push origin $(git rev-parse --abbrev-ref HEAD) v$version"
    else
        echo
        echo "Files updated. Nothing committed."
        echo "Review with 'git diff', then:"
        echo "    scripts/release.sh set $version --commit"
    fi
}

case "${1:-}" in
    current) cmd_current ;;
    check)   shift; cmd_check "${1:-}" ;;
    set)     shift; cmd_set "${1:-}" "${2:-}" ;;
    *)
        cat >&2 <<'USAGE'
usage: scripts/release.sh <command>

  current              print the workspace version
  check [vX.Y.Z]       assert every version reference agrees; with a tag
                       argument, also assert the tag matches
  set X.Y.Z            rewrite every version reference
  set X.Y.Z --commit   ...and commit + tag (does not push)
USAGE
        exit 2
        ;;
esac
